//! Accounts: credential *references* for a harness run, never secret material.
//!
//! **The sharpest invariant in this module: `am`'s account store on disk holds
//! only env-var NAMES, a base URL, a helper-command string, and/or a path to a
//! private home dir — never a secret value.** A secret value may be read
//! transiently from the environment at launch (see `harness::claude::provision`)
//! and placed into the child process's env in memory; it is never written to
//! disk by `am`.
//!
//! This mirrors the shape of [`crate::registry`]: a trait ([`AccountStore`]) so
//! embedders can back it with whatever they like, and a filesystem-backed
//! implementation ([`FsAccountStore`]) for the CLI.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};

use crate::Result;

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
    /// A private config/credentials directory. The child process gets this
    /// via `HOME`, so a harness's own OAuth/keychain credential store
    /// (independent of `am`'s injected skills/mcp config) can be kept
    /// per-account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    /// Non-secret metadata captured at login (auth type, plan tier, redacted
    /// identity). Never a token/secret value. Empty unless populated by an
    /// `am account login` capture.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub captured: BTreeMap<String, String>,
}

/// A source of [`Account`]s, resolved by id.
pub trait AccountStore {
    /// All accounts, sorted by id.
    fn accounts(&self) -> Result<Vec<Account>>;
    /// One account by exact id.
    fn account(&self, id: &str) -> Result<Option<Account>> {
        Ok(self.accounts()?.into_iter().find(|a| a.id == id))
    }
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
                    bail!(
                        "account entry in {} is missing 'id'",
                        toml_path.display()
                    );
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
        assert_eq!(personal.base_url.as_deref(), Some("https://api.anthropic.com"));
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
}
