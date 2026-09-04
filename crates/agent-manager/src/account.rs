//! Accounts: credential *references* for a harness run, never secret material
//! in the account index.
//!
//! **The sharpest invariant in this module: `am`'s account *index* on disk
//! (`accounts.toml` / per-id toml) holds only env-var NAMES, a base URL, a
//! helper-command string, and/or a path to a private home dir — never a
//! secret value.** Secret values may:
//! - be read transiently from the environment at launch and placed into the
//!   child process's env in memory;
//! - live under an account **home** as harness-native credential files
//!   (written by `am account login`, or by `am account import --write`
//!   materializing a macOS Keychain Claude OAuth blob into
//!   `accounts/default/`).
//!
//! The index still never stores tokens; the home is the same seam
//! `seed_login` already uses at provision time.
//!
//! This mirrors the shape of [`crate::registry`]: a trait ([`AccountStore`]) so
//! embedders can back it with whatever they like, and a filesystem-backed
//! implementation ([`FsAccountStore`]) for the CLI.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};

use crate::Result;
use crate::credentials::{CredentialBlob, Validity, credential_validity};
use crate::harness::Harness;
use crate::source::Source;

/// A named credential *reference* for a harness run.
///
/// Holds only references, never secrets: env-var NAMES (whose values are read
/// transiently at launch time and passed through to the child process), a
/// provider base URL, a helper-command string (never run by `am` itself —
/// only wired into the harness's native key-helper slot), and/or a path to a
/// private home directory. No field here can hold a secret value.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct Account {
    /// Stable account identifier (the store key). Required for entries inline
    /// in `accounts.toml`; defaults to the file stem for per-file entries.
    #[serde(default)]
    pub id: String,
    /// NAME of an env var whose value is passed through to the harness's
    /// native API-key env var (e.g. `ANTHROPIC_API_KEY`) at launch. The value
    /// itself is read transiently from `am`'s environment at launch time and
    /// is never written to disk by `am`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// NAME of an env var whose value is passed through to the harness's
    /// native auth-token env var (e.g. `ANTHROPIC_AUTH_TOKEN`) at launch.
    /// Same never-written-to-disk rule as [`Self::api_key_env`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token_env: Option<String>,
    /// Provider base URL (e.g. a gateway/proxy endpoint), passed through to
    /// the harness's native base-URL env var.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// A command whose stdout yields the key, wired into the harness's native
    /// key-helper slot (e.g. Claude Code's `apiKeyHelper` setting). `am`
    /// never runs this command or sees its output — the harness does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    /// A private directory holding this account's captured harness login
    /// (written by `am account login`, laid out HOME-relative, e.g.
    /// `<home>/.claude/.credentials.json` + `<home>/.claude.json`). At launch a
    /// harness *seeds* the relevant login files from here into its relocated
    /// config dir (e.g. `CLAUDE_CONFIG_DIR`) — it does **not** override the
    /// child's `HOME`, which would strip the user's toolchain (nvm/mise/pyenv,
    /// shell rc, PATH shims). Harnesses with no config-dir lever (grok) are the
    /// exception and must relocate `HOME`; see `_docs/profiles.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    /// Non-secret metadata captured at login (auth type, plan tier, redacted
    /// identity). Never a token/secret value. Empty unless populated by an
    /// `am account login` capture.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub captured: BTreeMap<String, String>,
}

/// A source of [`Account`]s, resolved by id — plus the credential
/// capture/seed seam.
///
/// Reads ([`accounts`](Self::accounts)/[`account`](Self::account)) and the
/// login capture/seed methods all go through this trait, so an embedder can
/// back accounts with a database while the CLI uses [`FsAccountStore`]. The
/// login methods default to a filesystem-friendly behavior or a read-only
/// error, so an in-memory or reference-only store implements only what it
/// needs. See `_docs/am-as-library.md`.
pub trait AccountStore {
    /// All accounts, sorted by id.
    fn accounts(&self) -> Result<Vec<Account>>;
    /// One account by exact id.
    fn account(&self, id: &str) -> Result<Option<Account>> {
        Ok(self.accounts()?.into_iter().find(|a| a.id == id))
    }

    /// The account's captured-login content, materialized into a harness's
    /// relocated config dir at launch (see [`crate::harness::seed_login`]).
    ///
    /// Default: derive a [`Source::Dir`] from the account's `home` reference —
    /// correct for any store whose [`Account`] carries an on-disk home
    /// (including [`FsAccountStore`]). A database-backed store that keeps
    /// credential *bytes* rather than a home dir overrides this to return a
    /// [`Source::Files`]. `None` means no captured login (env/key/helper
    /// accounts, or an unknown id).
    fn login_source(&self, id: &str) -> Result<Option<Source>> {
        Ok(self.account(id)?.and_then(|a| a.home).map(Source::Dir))
    }

    /// A real directory to run an interactive `am account login` in.
    ///
    /// The harness login is a real subprocess that must write to a real dir
    /// (the same physical constraint as the run dir). [`FsAccountStore`]
    /// returns the persistent per-account home; a database-backed store returns
    /// a temp dir it will read back in [`capture_login`](Self::capture_login).
    /// Default: a read-only error.
    fn login_home(&self, _id: &str) -> Result<PathBuf> {
        bail!("this account store does not support login capture")
    }

    /// Persist a login captured under [`login_home`](Self::login_home): record
    /// the account and store the credential `files` (paths relative to `from`).
    ///
    /// [`FsAccountStore`] points the account's `home` at `from` and saves the
    /// reference (the harness already wrote the files there); a database-backed
    /// store reads each file's bytes from `from` and stores them. This is the
    /// credential **write seam** — also where copy-back-on-exit for refreshed
    /// OAuth tokens would hook in (see `_docs/open-points.md` §9). Default: a
    /// read-only error.
    fn capture_login(&self, _id: &str, _from: &Path, _files: &[PathBuf]) -> Result<()> {
        bail!("this account store does not support login capture")
    }

    /// Rename an account: move its home directory and its on-disk record so
    /// future lookups address it as `to`. An account is a home shared by
    /// every harness logged in there, so every captured login moves with it
    /// in one step — there is no per-harness rename. Default: a read-only
    /// error.
    fn rename_account(&self, _from: &str, _to: &str) -> Result<()> {
        bail!("this account store does not support renaming accounts")
    }

    /// Delete an account entirely: its home directory (every harness's
    /// captured login) and its on-disk record. Default: a read-only error.
    fn delete_account(&self, _id: &str) -> Result<()> {
        bail!("this account store does not support deleting accounts")
    }

    /// Sign one harness out of an account: remove the credential `files`
    /// (paths relative to the account home, the same shape
    /// [`capture_login`](Self::capture_login) already takes — the harness
    /// names its own files, so the store never needs to know a harness's
    /// layout). The account and every other harness's login under the same
    /// home survive. Default: a read-only error.
    fn sign_out(&self, _id: &str, _files: &[PathBuf]) -> Result<()> {
        bail!("this account store does not support signing out")
    }
}

/// The validity of `harness`'s stored login inside an account home, computed
/// from whatever expiry the credential names about itself.
///
/// Reads the files the harness itself declares in
/// [`crate::harness::ConfigAnchor::login_seed`] — the caller names no path,
/// which is what keeps an embedder from having to know where a harness keeps
/// its credential. A file that is absent contributes nothing, so an account
/// with no login for this harness reads as [`Validity::Empty`].
///
/// Local and offline: this is what the credential claims, not what the
/// provider would say. A token revoked early still reads as valid.
pub fn login_validity(harness: &dyn Harness, home: &Path, now_ms: i64) -> Validity {
    let mut blobs = Vec::new();
    for seed in &harness.config_anchor().login_seed {
        let path = home.join(&seed.src);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = seed
            .src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| seed.src.to_string_lossy().into_owned());
        blobs.push(CredentialBlob {
            name,
            rel_path: seed.src.clone(),
            bytes,
        });
    }
    credential_validity(&blobs, now_ms)
}

/// An [`AccountStore`] with no accounts — the default for lib-mode embedders
/// and for the CLI when no accounts root is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyAccountStore;

impl AccountStore for EmptyAccountStore {
    fn accounts(&self) -> Result<Vec<Account>> {
        Ok(Vec::new())
    }
}

/// A filesystem-backed account store rooted at an accounts directory.
///
/// Two layers, both optional, combined:
/// - `accounts.toml` with inline `[[account]]` entries (each requires `id`).
/// - Per-file `<id>.toml` (the `id` field defaults to the file stem if absent).
///
/// An id appearing in both layers (or twice within a layer) is a load-time
/// error, mirroring [`crate::registry::FsRegistry`]'s MCP-id collision rule.
#[derive(Debug, Clone)]
pub struct FsAccountStore {
    root: PathBuf,
}

impl FsAccountStore {
    /// Create a store rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsAccountStore { root: root.into() }
    }

    /// Persist `account` as a per-file `<id>.toml` under the store root
    /// (creating the root). Overwrites an existing per-file entry. Holds only
    /// references/metadata — never a secret value (same invariant as the rest
    /// of this module).
    pub fn save(&self, account: &Account) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating {}", self.root.display()))?;
        let path = self.root.join(format!("{}.toml", account.id));
        let body = toml::to_string_pretty(account)
            .with_context(|| format!("serializing account '{}'", account.id))?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }
}

/// Parsed structure for `accounts.toml`.
#[derive(Debug, serde::Deserialize, Default)]
struct AccountsToml {
    /// Inline account definitions.
    #[serde(default)]
    account: Vec<Account>,
}

impl AccountStore for FsAccountStore {
    fn accounts(&self) -> Result<Vec<Account>> {
        let mut entries = Vec::new();
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();

        // Inline entries from accounts.toml.
        let toml_path = self.root.join("accounts.toml");
        if toml_path.exists() {
            let content = std::fs::read_to_string(&toml_path)
                .with_context(|| format!("reading {}", toml_path.display()))?;
            let parsed: AccountsToml = toml::from_str(&content)
                .with_context(|| format!("parsing {}", toml_path.display()))?;

            for acct in parsed.account {
                if acct.id.is_empty() {
                    bail!("account entry in {} is missing 'id'", toml_path.display());
                }
                if !seen_ids.insert(acct.id.clone()) {
                    bail!(
                        "account id collision: '{}' appears more than once in {}",
                        acct.id,
                        toml_path.display()
                    );
                }
                entries.push(acct);
            }
        }

        // Per-file entries: <id>.toml (excluding accounts.toml itself).
        if self.root.is_dir() {
            for entry in std::fs::read_dir(&self.root)
                .with_context(|| format!("reading directory {}", self.root.display()))?
            {
                let entry = entry?;
                let path = entry.path();

                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                if path.file_name().and_then(|n| n.to_str()) == Some("accounts.toml") {
                    continue;
                }

                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("invalid account file name: {}", path.display()))?;

                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let mut acct: Account = toml::from_str(&content)
                    .with_context(|| format!("parsing {}", path.display()))?;
                if acct.id.is_empty() {
                    acct.id = stem;
                }

                if !seen_ids.insert(acct.id.clone()) {
                    bail!(
                        "account id collision: '{}' appears in both accounts.toml and a per-file entry ({})",
                        acct.id,
                        path.display()
                    );
                }
                entries.push(acct);
            }
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }

    fn login_home(&self, id: &str) -> Result<PathBuf> {
        let home = self.root.join(id);
        std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
        Ok(home)
    }

    fn capture_login(&self, id: &str, from: &Path, _files: &[PathBuf]) -> Result<()> {
        // The harness already wrote its credential files under `from` (the
        // per-account home); persist the reference by pointing `home` at it.
        let mut acct = self.account(id)?.unwrap_or(Account {
            id: id.to_string(),
            ..Default::default()
        });
        acct.home = Some(from.to_path_buf());
        self.save(&acct)?;
        Ok(())
    }

    fn rename_account(&self, from: &str, to: &str) -> Result<()> {
        validate_account_name(to)?;

        let acct = self
            .account(from)?
            .ok_or_else(|| anyhow!("no account named '{from}'"))?;

        let from_record = self.root.join(format!("{from}.toml"));
        if !from_record.is_file() {
            bail!(
                "account '{from}' is declared inline in accounts.toml; edit it there \
                 directly instead of renaming"
            );
        }

        let to_home = self.root.join(to);
        let to_record = self.root.join(format!("{to}.toml"));
        if to_home.exists() || to_record.exists() || self.account(to)?.is_some() {
            bail!("an account named '{to}' already exists");
        }

        let from_home = self.root.join(from);
        let home_moved = if from_home.is_dir() {
            std::fs::rename(&from_home, &to_home).with_context(|| {
                format!("renaming {} -> {}", from_home.display(), to_home.display())
            })?;
            true
        } else {
            false
        };

        let mut new_acct = acct;
        new_acct.id = to.to_string();
        if home_moved {
            new_acct.home = Some(to_home);
        }
        self.save(&new_acct)?;
        std::fs::remove_file(&from_record)
            .with_context(|| format!("removing {}", from_record.display()))?;
        Ok(())
    }

    fn delete_account(&self, id: &str) -> Result<()> {
        self.account(id)?
            .ok_or_else(|| anyhow!("no account named '{id}'"))?;

        let record = self.root.join(format!("{id}.toml"));
        if !record.is_file() {
            bail!(
                "account '{id}' is declared inline in accounts.toml; edit it there \
                 directly instead of deleting"
            );
        }

        let home = self.root.join(id);
        if home.is_dir() {
            std::fs::remove_dir_all(&home)
                .with_context(|| format!("removing {}", home.display()))?;
        }
        std::fs::remove_file(&record).with_context(|| format!("removing {}", record.display()))?;
        Ok(())
    }

    fn sign_out(&self, id: &str, files: &[PathBuf]) -> Result<()> {
        self.account(id)?
            .ok_or_else(|| anyhow!("no account named '{id}'"))?;

        let home = self.root.join(id);
        for rel in files {
            let path = home.join(rel);
            if path.is_file() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
            prune_empty_ancestors(path.parent(), &home)?;
        }
        Ok(())
    }
}

/// Reject a name unfit to become a filesystem entry under the accounts root:
/// empty/whitespace-only, `.`/`..`, or containing a path separator or a nul
/// byte. A trust boundary — a name arriving from an embedder's UI must not be
/// able to escape the accounts root.
fn validate_account_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("account name cannot be empty or whitespace-only");
    }
    if name == "." || name == ".." {
        bail!("account name cannot be '{name}'");
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        bail!("account name cannot contain a path separator or a null byte, got '{name}'");
    }
    Ok(())
}

/// Remove `dir` and walk up removing each now-empty ancestor, stopping at
/// (never removing) `home` itself — so a harness's directory left holding
/// nothing after [`AccountStore::sign_out`] does not read as a login, while
/// the account home always survives.
fn prune_empty_ancestors(dir: Option<&Path>, home: &Path) -> Result<()> {
    let mut dir = dir;
    while let Some(d) = dir {
        if d == home || !d.starts_with(home) {
            break;
        }
        let is_empty = std::fs::read_dir(d)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            break;
        }
        std::fs::remove_dir(d).with_context(|| format!("removing {}", d.display()))?;
        dir = d.parent();
    }
    Ok(())
}

/// The default accounts root: `~/.config/agent-manager/accounts` on all
/// platforms — the same base dir as the config file
/// ([`crate::settings::default_config_dir`]), so `config.toml` and `accounts/`
/// live together. Overridable by `AM_ACCOUNTS` (see [`resolve_accounts_root`]).
pub fn default_accounts_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("accounts"))
}

/// Resolve the accounts root from (highest first): an explicit path, the
/// `AM_ACCOUNTS` env var, then the default. Returns `None` if none apply.
pub fn resolve_accounts_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_ACCOUNTS").ok().map(PathBuf::from))
        .or_else(default_accounts_root)
}

/// Account id for the ambient / imported default credentials.
///
/// Keychain import (`am account import --write` on macOS) always materializes
/// under this id (`accounts/default/` + `default.toml`) and sets
/// `[defaults].account = "default"` so bare `am claude` reuses it. Do not
/// invent a parallel id (e.g. `claude-credentials-home`) for the ambient
/// Claude session — that path is `default` only.
pub const DEFAULT_ACCOUNT_ID: &str = "default";

/// macOS Keychain *service* name Claude Code uses for the OAuth blob
/// (verified against Claude Code docs / field reports).
pub const CLAUDE_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Normalize and validate a Claude credentials JSON blob (as stored in
/// Keychain or `~/.claude/.credentials.json`).
///
/// Accepts either the full file shape `{"claudeAiOauth":{…}}` or a bare
/// oauth object `{accessToken, refreshToken, …}` (wrapped on output).
/// Rejects empty / missing tokens so we never materialize a useless stub.
pub fn normalize_claude_credentials_json(raw: &[u8]) -> Result<Vec<u8>> {
    let trimmed = trim_trailing_whitespace(raw);
    if trimmed.is_empty() {
        bail!("Claude credentials blob is empty");
    }
    let value: serde_json::Value =
        serde_json::from_slice(trimmed).context("parsing Claude credentials JSON")?;
    let oauth = match value.get("claudeAiOauth") {
        Some(inner) => inner.clone(),
        None if value.get("accessToken").is_some() || value.get("refreshToken").is_some() => value,
        None => {
            bail!("Claude credentials JSON missing `claudeAiOauth` (and not a bare oauth object)")
        }
    };
    let access = oauth
        .get("accessToken")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let refresh = oauth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if access.is_empty() && refresh.is_empty() {
        bail!(
            "Claude credentials have empty accessToken and refreshToken \
             (Keychain entry present but unusable — re-login with `claude auth login`)"
        );
    }
    let doc = serde_json::json!({ "claudeAiOauth": oauth });
    Ok(serde_json::to_vec_pretty(&doc)?)
}

/// Read Claude Code's OAuth credentials from the macOS Keychain.
///
/// Runs `security find-generic-password -a $USER -s 'Claude Code-credentials' -w`.
/// Returns normalized pretty-printed JSON suitable for
/// `.claude/.credentials.json`. Errors on non-macOS, missing entry, empty
/// tokens, or Keychain ACL denial.
///
/// The first successful read may show a system allow-prompt for `security` /
/// the calling binary; headless sessions can fail with interaction-not-allowed.
pub fn read_claude_keychain_credentials() -> Result<Vec<u8>> {
    #[cfg(not(target_os = "macos"))]
    {
        bail!("Claude Keychain import is only supported on macOS");
    }
    #[cfg(target_os = "macos")]
    {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .context(
                "USER/LOGNAME not set (needed as Keychain account attribute for Claude credentials)",
            )?;
        let output = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-a",
                &user,
                "-s",
                CLAUDE_KEYCHAIN_SERVICE,
                "-w",
            ])
            .output()
            .context("running `security find-generic-password` (is the security CLI available?)")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Keychain entry {:?} (account {:?}) not readable ({}): {}",
                CLAUDE_KEYCHAIN_SERVICE,
                user,
                output.status,
                stderr.trim()
            );
        }
        normalize_claude_credentials_json(&output.stdout)
    }
}

/// Write a Claude login layout under `home` for later [`crate::harness::seed_login`]:
/// - `<home>/.claude/.credentials.json` (from `creds`, mode `0600` on Unix)
/// - `<home>/.claude.json` copied from the real user home when present (identity)
///
/// `creds` must already be a validated full-file JSON blob
/// ([`normalize_claude_credentials_json`]).
pub fn materialize_claude_login_home(home: &Path, creds: &[u8]) -> Result<()> {
    let cred_path = home.join(".claude").join(".credentials.json");
    if let Some(parent) = cred_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&cred_path, creds)
        .with_context(|| format!("writing {}", cred_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&cred_path, perms)
            .with_context(|| format!("chmod 600 {}", cred_path.display()))?;
    }

    // Identity / onboarding metadata — same optional companion file as
    // Claude::login capture. Prefer real $HOME over directories crate so a
    // relocated test HOME still works.
    let real_home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(real_home) = real_home {
        let src = real_home.join(".claude.json");
        if src.is_file() {
            let dst = home.join(".claude.json");
            std::fs::copy(&src, &dst)
                .with_context(|| format!("copying {} → {}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

/// Materialize the Keychain Claude session into `accounts/default/` and
/// record the [`DEFAULT_ACCOUNT_ID`] account pointing at that home.
///
/// Returns the account. Does **not** set `[defaults].account` — the CLI
/// import path does that when appropriate.
pub fn import_default_claude_from_keychain(accounts_root: &Path) -> Result<Account> {
    let creds = read_claude_keychain_credentials()?;
    let home = accounts_root.join(DEFAULT_ACCOUNT_ID);
    materialize_claude_login_home(&home, &creds)?;
    let acct = Account {
        id: DEFAULT_ACCOUNT_ID.to_string(),
        home: Some(home),
        captured: claude_keychain_captured(&creds),
        ..Default::default()
    };
    std::fs::create_dir_all(accounts_root)
        .with_context(|| format!("creating {}", accounts_root.display()))?;
    FsAccountStore::new(accounts_root).save(&acct)?;
    Ok(acct)
}

/// Record the `default` Claude account index entry from Keychain `creds`
/// **without** materializing any plaintext credential files — for secure
/// credential engines (`os`/`keychain`) that hold the secret bytes themselves.
/// Mirrors [`import_default_claude_from_keychain`] minus the file-home
/// materialization, so `home` is `None` (the [`crate::credentials::SecretStore`]
/// serves the login at run time).
pub fn record_default_claude_from_keychain(accounts_root: &Path, creds: &[u8]) -> Result<Account> {
    let acct = Account {
        id: DEFAULT_ACCOUNT_ID.to_string(),
        home: None,
        captured: claude_keychain_captured(creds),
        ..Default::default()
    };
    std::fs::create_dir_all(accounts_root)
        .with_context(|| format!("creating {}", accounts_root.display()))?;
    FsAccountStore::new(accounts_root).save(&acct)?;
    Ok(acct)
}

/// Non-secret `captured` metadata for a Keychain-imported Claude login.
fn claude_keychain_captured(creds: &[u8]) -> BTreeMap<String, String> {
    let mut captured = BTreeMap::new();
    captured.insert("source".to_string(), "macos-keychain".to_string());
    captured.insert("service".to_string(), CLAUDE_KEYCHAIN_SERVICE.to_string());
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(creds)
        && let Some(sub) = v
            .pointer("/claudeAiOauth/subscriptionType")
            .and_then(|s| s.as_str())
    {
        captured.insert("subscriptionType".to_string(), sub.to_string());
    }
    captured
}

fn trim_trailing_whitespace(raw: &[u8]) -> &[u8] {
    let mut end = raw.len();
    while end > 0 && matches!(raw[end - 1], b' ' | b'\n' | b'\r' | b'\t') {
        end -= 1;
    }
    &raw[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fs_account_store_parses_inline_and_per_file_entries() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        fs::write(
            root.join("accounts.toml"),
            r#"
[[account]]
id = "personal"
api_key_env = "PERSONAL_ANTHROPIC_KEY"
base_url = "https://api.anthropic.com"

[[account]]
id = "work"
auth_token_env = "WORK_ANTHROPIC_TOKEN"
helper = "work-key-helper"
"#,
        )?;

        fs::write(
            root.join("sandbox.toml"),
            r#"
home = "/private/sandbox-home"
"#,
        )?;

        let store = FsAccountStore::new(root);
        let accounts = store.accounts()?;

        let ids: Vec<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["personal", "sandbox", "work"]);

        let personal = accounts.iter().find(|a| a.id == "personal").unwrap();
        assert_eq!(
            personal.api_key_env.as_deref(),
            Some("PERSONAL_ANTHROPIC_KEY")
        );
        assert_eq!(
            personal.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(personal.auth_token_env.is_none());
        assert!(personal.helper.is_none());
        assert!(personal.home.is_none());

        let work = accounts.iter().find(|a| a.id == "work").unwrap();
        assert_eq!(work.auth_token_env.as_deref(), Some("WORK_ANTHROPIC_TOKEN"));
        assert_eq!(work.helper.as_deref(), Some("work-key-helper"));

        let sandbox = accounts.iter().find(|a| a.id == "sandbox").unwrap();
        assert_eq!(sandbox.home, Some(PathBuf::from("/private/sandbox-home")));

        temp.close()?;
        Ok(())
    }

    #[test]
    fn fs_account_store_collision_between_inline_and_per_file_is_an_error() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        fs::write(
            root.join("accounts.toml"),
            r#"
[[account]]
id = "work"
api_key_env = "WORK_KEY"
"#,
        )?;
        fs::write(root.join("work.toml"), "api_key_env = \"OTHER_KEY\"\n")?;

        let store = FsAccountStore::new(root);
        let err = store.accounts().expect_err("should error on collision");
        assert!(err.to_string().contains("collision"), "message was: {err}");

        temp.close()?;
        Ok(())
    }

    #[test]
    fn fs_account_store_missing_id_returns_none() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        fs::write(
            root.join("accounts.toml"),
            r#"
[[account]]
id = "work"
api_key_env = "WORK_KEY"
"#,
        )?;

        let store = FsAccountStore::new(root);
        assert!(store.account("missing")?.is_none());
        assert!(store.account("work")?.is_some());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn empty_account_store_has_no_accounts() {
        let store = EmptyAccountStore;
        assert!(store.accounts().unwrap().is_empty());
        assert!(store.account("anything").unwrap().is_none());
    }

    #[test]
    fn fs_account_store_save_round_trips_id_and_home() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path().join("accounts");

        let account = Account {
            id: "cap".to_string(),
            home: Some(PathBuf::from("/private/cap-home")),
            ..Default::default()
        };

        let store = FsAccountStore::new(&root);
        let path = store.save(&account)?;
        assert!(path.exists());

        let loaded = FsAccountStore::new(&root)
            .account("cap")?
            .expect("saved account should be found");
        assert_eq!(loaded.id, "cap");
        assert_eq!(loaded.home, Some(PathBuf::from("/private/cap-home")));

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_moves_home_and_record_and_reloads_under_new_id() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);

        let home = store.login_home("work")?;
        fs::write(home.join("token.json"), "secret")?;
        store.capture_login("work", &home, &[])?;

        store.rename_account("work", "personal")?;

        assert!(store.account("work")?.is_none());
        let renamed = store
            .account("personal")?
            .expect("renamed account should be found");
        assert_eq!(renamed.id, "personal");
        let new_home = root.join("personal");
        assert_eq!(renamed.home, Some(new_home.clone()));
        assert!(new_home.join("token.json").is_file());
        assert!(!root.join("work").exists());
        assert!(!root.join("work.toml").exists());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_to_existing_name_errors() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = FsAccountStore::new(temp.path());
        store.save(&acct_with_id("work"))?;
        store.save(&acct_with_id("personal"))?;

        let err = store
            .rename_account("work", "personal")
            .expect_err("should error renaming onto an existing account");
        assert!(err.to_string().contains("already exists"), "{err}");

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_rejects_separator_or_dotdot_and_touches_nothing() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);
        let home = store.login_home("work")?;
        fs::write(home.join("token.json"), "secret")?;
        store.capture_login("work", &home, &[])?;

        assert!(store.rename_account("work", "../escaped").is_err());
        assert!(store.rename_account("work", "sub/dir").is_err());
        assert!(store.rename_account("work", "..").is_err());
        assert!(store.rename_account("work", "  ").is_err());

        // Nothing was touched: the original account is unchanged.
        assert!(root.join("work").is_dir());
        assert!(root.join("work.toml").is_file());
        assert!(store.account("work")?.is_some());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_of_missing_errors() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FsAccountStore::new(temp.path());
        assert!(store.rename_account("ghost", "new-name").is_err());
    }

    #[test]
    fn delete_account_removes_home_and_record() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);
        let home = store.login_home("work")?;
        fs::write(home.join("token.json"), "secret")?;
        store.capture_login("work", &home, &[])?;

        store.delete_account("work")?;

        assert!(!root.join("work").exists());
        assert!(!root.join("work.toml").exists());
        assert!(store.account("work")?.is_none());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn delete_account_of_missing_errors() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FsAccountStore::new(temp.path());
        assert!(store.delete_account("ghost").is_err());
    }

    #[test]
    fn sign_out_removes_only_named_files_and_leaves_home_and_sibling_intact() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);
        let home = store.login_home("work")?;
        fs::create_dir_all(home.join(".claude"))?;
        fs::write(home.join(".claude/.credentials.json"), "claude-secret")?;
        fs::create_dir_all(home.join(".codex"))?;
        fs::write(home.join(".codex/auth.json"), "codex-secret")?;
        store.capture_login("work", &home, &[])?;

        store.sign_out("work", &[PathBuf::from(".claude/.credentials.json")])?;

        assert!(!home.join(".claude/.credentials.json").exists());
        // The now-empty harness dir was pruned...
        assert!(!home.join(".claude").exists());
        // ...but the sibling harness's login and the home itself survive.
        assert!(home.join(".codex/auth.json").is_file());
        assert!(home.is_dir());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn login_validity_empty_for_home_with_no_credential() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let harness = crate::harness::Claude::new();
        let v = login_validity(&harness, temp.path(), 1_000);
        assert_eq!(v, Validity::Empty);
        temp.close()?;
        Ok(())
    }

    #[test]
    fn login_validity_valid_and_expired_for_planted_credential() -> Result<()> {
        // Write the credential file directly (not via
        // `materialize_claude_login_home`, which also copies a real
        // `~/.claude.json` companion file when one exists on the running
        // machine — this test wants only the one planted blob under test).
        let temp = tempfile::TempDir::new()?;
        let home = temp.path();
        let harness = crate::harness::Claude::new();
        let cred_path = home.join(".claude").join(".credentials.json");
        fs::create_dir_all(cred_path.parent().unwrap())?;

        let future = 5_000_000_000_000i64;
        let creds =
            format!("{{\"claudeAiOauth\":{{\"accessToken\":\"a\",\"expiresAt\":{future}}}}}");
        fs::write(&cred_path, &creds)?;
        assert_eq!(
            login_validity(&harness, home, 1_000_000_000_000),
            Validity::Valid {
                expires_at_ms: Some(future)
            }
        );

        let past = 1_000_000_000_000i64;
        let expired =
            format!("{{\"claudeAiOauth\":{{\"accessToken\":\"a\",\"expiresAt\":{past}}}}}");
        fs::write(&cred_path, &expired)?;
        assert_eq!(
            login_validity(&harness, home, 2_000_000_000_000),
            Validity::Expired {
                expires_at_ms: past
            }
        );

        temp.close()?;
        Ok(())
    }

    fn acct_with_id(id: &str) -> Account {
        Account {
            id: id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn normalize_claude_credentials_accepts_full_file_shape() {
        let raw = br#"{
            "claudeAiOauth": {
                "accessToken": "at-secret",
                "refreshToken": "rt-secret",
                "expiresAt": 123,
                "subscriptionType": "pro"
            }
        }"#;
        let out = normalize_claude_credentials_json(raw).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            v.pointer("/claudeAiOauth/accessToken")
                .and_then(|x| x.as_str()),
            Some("at-secret")
        );
    }

    #[test]
    fn normalize_claude_credentials_wraps_bare_oauth_object() {
        let raw = br#"{"accessToken":"a","refreshToken":"r"}"#;
        let out = normalize_claude_credentials_json(raw).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(v.get("claudeAiOauth").is_some());
        assert_eq!(
            v.pointer("/claudeAiOauth/accessToken")
                .and_then(|x| x.as_str()),
            Some("a")
        );
    }

    #[test]
    fn normalize_claude_credentials_rejects_empty_tokens() {
        let raw = br#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","expiresAt":0}}"#;
        let err = normalize_claude_credentials_json(raw)
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn normalize_claude_credentials_trims_trailing_newline() {
        let mut raw = br#"{"claudeAiOauth":{"accessToken":"x","refreshToken":"y"}}"#.to_vec();
        raw.push(b'\n');
        assert!(normalize_claude_credentials_json(&raw).is_ok());
    }

    #[test]
    fn materialize_claude_login_home_writes_credentials_layout() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let home = temp.path().join("default");
        let creds = normalize_claude_credentials_json(
            br#"{"claudeAiOauth":{"accessToken":"tok","refreshToken":"ref"}}"#,
        )?;
        materialize_claude_login_home(&home, &creds)?;
        let path = home.join(".claude/.credentials.json");
        assert!(path.is_file());
        let body = fs::read_to_string(&path)?;
        assert!(body.contains("tok"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "credentials file should be 0600, got {mode:o}");
        }
        Ok(())
    }
}
