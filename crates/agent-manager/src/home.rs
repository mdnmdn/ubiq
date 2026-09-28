//! The persistent harness config home, keyed by **account** (`D194`).
//!
//! One directory per `(account, harness)`: every run that names that account — from any
//! definition, from any project, concurrently — uses it as the harness's own config home,
//! read-write, and the harness owns refresh and cross-process sharing. A run that names no
//! account uses the harness's own default config in place ([`crate::spec::ConfigStrategy::Native`])
//! and nothing here is consulted.
//!
//! Nothing in this module captures, copies or roams a token. The login is performed *into* a
//! home by [`crate::harness::Harness::login_home`] and read back by nobody;
//! [`login_validity`] is the single, deliberate exception, and reads in place.
//!
//! The layout is `<root>/<account>/<harness>/`. The root is the embedder's: Ubiq constructs a
//! [`HomeStore`] with its own, and the CLI uses [`resolve_homes_root`].

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;
use crate::credentials::{CredentialBlob, Validity, credential_validity};
use crate::harness::Harness;

/// The file a finished sign-in leaves in a home. Empty: it is a fact, not a credential.
const SIGNED_IN_MARKER: &str = ".ubiq-signed-in";

/// The account-keyed config homes under one root.
///
/// Holds nothing but the root: every answer is computed from it, and the only method that
/// creates a home is [`HomeStore::mark_signed_in`] (otherwise that is
/// [`crate::provision::prepare_home`]'s job, once, at the point of use).
#[derive(Debug, Clone)]
pub struct HomeStore {
    root: PathBuf,
}

impl HomeStore {
    /// A store over `root`. The directory need not exist.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        HomeStore { root: root.into() }
    }

    /// The root every home sits under.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/<account>/<harness>` — computed, never created here.
    ///
    /// `None` when `account` is empty or is not a single, safe path segment (it holds a
    /// separator or a nul byte, or is `.` or `..`), so a name a person typed can never become a
    /// path that escapes the root.
    pub fn home(&self, account: &str, harness: &str) -> Option<PathBuf> {
        if !safe_segment(account) {
            return None;
        }
        Some(self.root.join(account).join(harness))
    }

    /// Whether this account is signed in to `harness`: a sign-in finished here
    /// ([`Self::mark_signed_in`]), or one of [`Harness::login_files`] is present under the home.
    ///
    /// **The marker comes first because the files are not where the login always is.** On macOS
    /// Claude Code keeps its login in the Keychain, under an item keyed by the home's path, and
    /// the home itself holds no credential file at all — so a reading that only looked at files
    /// would call a perfectly signed-in account signed in to nothing. What a completed sign-in
    /// knows is that it completed, and the marker is that fact, written by the embedder rather
    /// than hunted for. The files still count, because a login made by the harness's own first
    /// run leaves no marker.
    ///
    /// Existence of the directory alone is **not** a login —
    /// [`crate::provision::prepare_home`] creates it before anything signs in.
    pub fn signed_in(&self, harness: &dyn Harness, account: &str) -> bool {
        let Some(home) = self.home(account, &harness.id()) else {
            return false;
        };
        home.join(SIGNED_IN_MARKER).exists()
            || harness
                .login_files()
                .iter()
                .any(|rel| home.join(rel).exists())
    }

    /// Record that a sign-in into this home finished: an empty marker file beside whatever the
    /// harness keeps there. Creates the home if it is not there yet.
    ///
    /// It holds nothing — the credential is the harness's business, wherever the harness decided
    /// to put it. This only says a sign-in ran here and ended cleanly, which is the one thing a
    /// file in the home cannot be relied on to say (see [`Self::signed_in`]). A sign-out removes
    /// the home and the marker with it.
    pub fn mark_signed_in(&self, account: &str, harness: &str) -> Result<()> {
        let home = self.home(account, harness).ok_or_else(|| {
            anyhow::anyhow!("'{account}' is not a name a config home can be keyed by")
        })?;
        std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
        let marker = home.join(SIGNED_IN_MARKER);
        std::fs::write(&marker, b"").with_context(|| format!("writing {}", marker.display()))?;
        Ok(())
    }

    /// Whether a sign-in has been recorded for this home, ignoring what the harness left in it.
    /// The half of [`Self::signed_in`] a reader of a *validity* needs: a home marked but holding
    /// no readable login is signed in somewhere this cannot read, which is not the same answer as
    /// not signed in at all.
    pub fn marked_signed_in(&self, account: &str, harness: &str) -> bool {
        self.home(account, harness)
            .is_some_and(|home| home.join(SIGNED_IN_MARKER).exists())
    }

    /// The harness ids with a home directory under `account`, sorted. Empty when the account has
    /// no homes at all, or when its name is not a safe segment.
    pub fn harnesses(&self, account: &str) -> Vec<String> {
        if !safe_segment(account) {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(self.root.join(account)) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        ids
    }

    /// Move every home from one account name to another — what follows an account rename, so the
    /// logins stay with the account they were made for.
    ///
    /// A `from` with no homes is success (there is nothing to move). Fails if `to` already has
    /// homes, rather than merging two accounts' logins.
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let (src, dst) = (self.account_dir(from)?, self.account_dir(to)?);
        if !src.exists() {
            return Ok(());
        }
        if dst.exists() {
            anyhow::bail!(
                "account '{to}' already has harness homes at {}",
                dst.display()
            );
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::rename(&src, &dst)
            .with_context(|| format!("moving {} to {}", src.display(), dst.display()))?;
        Ok(())
    }

    /// Remove one harness's home under `account` — the sign-out. Removing a home that is not
    /// there is success, not an error.
    pub fn forget(&self, account: &str, harness: &str) -> Result<()> {
        let home = self
            .home(account, harness)
            .ok_or_else(|| anyhow::anyhow!("'{account}' is not a usable account name"))?;
        remove_dir(&home)
    }

    /// Remove every home under `account` — what follows deleting the account. Removing what is
    /// not there is success.
    pub fn delete(&self, account: &str) -> Result<()> {
        let dir = self.account_dir(account)?;
        remove_dir(&dir)
    }

    /// `<root>/<account>`, refusing a name that is not a single safe segment.
    fn account_dir(&self, account: &str) -> Result<PathBuf> {
        if !safe_segment(account) {
            anyhow::bail!("'{account}' is not a usable account name");
        }
        Ok(self.root.join(account))
    }
}

/// Remove `dir` and everything under it; absent is success.
fn remove_dir(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::new(e).context(format!("removing {}", dir.display()))),
    }
}

/// Whether `name` is fit to become one directory under the homes root: non-empty and not
/// whitespace-only, not `.` or `..`, and free of any path separator or nul byte. A trust
/// boundary — a name arriving from an embedder's UI must not be able to escape the root.
fn safe_segment(name: &str) -> bool {
    !name.trim().is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// The validity of `harness`'s login inside `home`, computed from whatever expiry the login
/// names about itself.
///
/// Reads the files the harness itself declares in [`Harness::login_files`] — the caller names no
/// path, which is what keeps an embedder from having to know where a harness keeps its
/// credential. A file that is absent contributes nothing, so a home with no login for this
/// harness reads as [`Validity::Empty`].
///
/// **This is the one token read `D194` keeps knowingly.** It is read in place: nothing is
/// copied, captured or written, and the reading exists only so a status line can say when a
/// login expires. It is temporary, to be dropped as soon as a usage/validity source exists that
/// does not read the login at all.
///
/// Local and offline: this is what the login claims, not what the provider would say. A token
/// revoked early still reads as valid.
pub fn login_validity(harness: &dyn Harness, home: &Path, now_ms: i64) -> Validity {
    let mut blobs = Vec::new();
    for rel in harness.login_files() {
        let Ok(bytes) = std::fs::read(home.join(&rel)) else {
            continue;
        };
        let name = rel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel.to_string_lossy().into_owned());
        blobs.push(CredentialBlob {
            name,
            rel_path: rel,
            bytes,
        });
    }
    credential_validity(&blobs, now_ms)
}

/// The default homes root: `~/.config/agent-manager/harness-homes`, beside `accounts/` and
/// `profiles/` ([`crate::settings::default_config_dir`]). Overridable by `AM_HOMES` (see
/// [`resolve_homes_root`]).
pub fn default_homes_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("harness-homes"))
}

/// Resolve the homes root from (highest first): an explicit path, the `AM_HOMES` env var, then
/// the default. Returns `None` if none apply.
pub fn resolve_homes_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_HOMES").ok().map(PathBuf::from))
        .or_else(default_homes_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Claude;

    fn store() -> (tempfile::TempDir, HomeStore) {
        let temp = tempfile::TempDir::new().unwrap();
        let store = HomeStore::new(temp.path().join("harness-homes"));
        (temp, store)
    }

    #[test]
    fn home_is_account_then_harness_and_is_not_created() {
        let (temp, store) = store();
        let home = store.home("work", "claude-code").expect("a safe name");
        assert_eq!(home, store.root().join("work").join("claude-code"));
        assert!(!home.exists(), "the accessor only computes the path");
        temp.close().unwrap();
    }

    #[test]
    fn an_unsafe_account_name_names_no_home() {
        let (temp, store) = store();
        for bad in ["", "   ", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(store.home(bad, "claude-code").is_none(), "accepted {bad:?}");
            assert!(store.forget(bad, "claude-code").is_err());
            assert!(store.delete(bad).is_err());
            assert!(store.harnesses(bad).is_empty());
        }
        temp.close().unwrap();
    }

    #[test]
    fn signed_in_reads_the_harnesss_own_login_file_not_the_directory() {
        let (temp, store) = store();
        let claude = Claude::new();
        assert!(!store.signed_in(&claude, "work"));

        let home = store.home("work", &claude.id()).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        assert!(
            !store.signed_in(&claude, "work"),
            "an empty prepared home is not a login"
        );

        std::fs::write(home.join(".credentials.json"), b"{}").unwrap();
        assert!(store.signed_in(&claude, "work"));
        temp.close().unwrap();
    }

    #[test]
    fn a_recorded_sign_in_reads_as_signed_in_with_no_login_file_at_all() {
        // The macOS case: Claude Code puts the login in the Keychain and leaves the home empty,
        // so only the marker can say the account is signed in.
        let (temp, store) = store();
        let claude = Claude::new();
        assert!(!store.marked_signed_in("work", &claude.id()));

        store.mark_signed_in("work", &claude.id()).unwrap();
        let home = store.home("work", &claude.id()).unwrap();
        assert!(home.is_dir(), "marking makes the home it marks");
        assert!(!home.join(".credentials.json").exists());
        assert!(store.signed_in(&claude, "work"));
        assert!(store.marked_signed_in("work", &claude.id()));

        store.forget("work", &claude.id()).unwrap();
        assert!(!store.signed_in(&claude, "work"), "signing out takes it");
        temp.close().unwrap();
    }

    #[test]
    fn harnesses_lists_the_homes_under_an_account_sorted() {
        let (temp, store) = store();
        assert!(store.harnesses("work").is_empty());
        for id in ["codex", "claude-code", "copilot"] {
            std::fs::create_dir_all(store.home("work", id).unwrap()).unwrap();
        }
        assert_eq!(store.harnesses("work"), ["claude-code", "codex", "copilot"]);
        temp.close().unwrap();
    }

    #[test]
    fn rename_moves_every_home_and_refuses_an_occupied_destination() {
        let (temp, store) = store();
        let from = store.home("work", "claude-code").unwrap();
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join(".credentials.json"), b"{}").unwrap();

        store.rename("work", "day-job").unwrap();
        assert!(!store.root().join("work").exists());
        assert!(
            store
                .home("day-job", "claude-code")
                .unwrap()
                .join(".credentials.json")
                .is_file()
        );

        // Nothing to move is success; a destination that already has homes is not.
        store.rename("nobody", "day-job").unwrap();
        std::fs::create_dir_all(store.home("other", "codex").unwrap()).unwrap();
        assert!(store.rename("other", "day-job").is_err());
        temp.close().unwrap();
    }

    #[test]
    fn forget_removes_one_harness_and_delete_removes_the_account() {
        let (temp, store) = store();
        for id in ["claude-code", "codex"] {
            std::fs::create_dir_all(store.home("work", id).unwrap()).unwrap();
        }

        store.forget("work", "codex").unwrap();
        assert_eq!(store.harnesses("work"), ["claude-code"]);
        // Removing what is not there is success.
        store.forget("work", "codex").unwrap();

        store.delete("work").unwrap();
        assert!(!store.root().join("work").exists());
        store.delete("work").unwrap();
        temp.close().unwrap();
    }

    #[test]
    fn login_validity_reads_the_expiry_the_login_states_about_itself() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path();
        let claude = Claude::new();
        let now = 1_700_000_000_000i64;

        // No file at all.
        assert_eq!(login_validity(&claude, home, now), Validity::Empty);

        let creds = home.join(".credentials.json");
        // Present, with a future expiry.
        let future = now + 60_000;
        std::fs::write(
            &creds,
            format!("{{\"claudeAiOauth\":{{\"expiresAt\":{future}}}}}"),
        )
        .unwrap();
        assert_eq!(
            login_validity(&claude, home, now),
            Validity::Valid {
                expires_at_ms: Some(future)
            }
        );

        // Present, with a past expiry.
        let past = now - 60_000;
        std::fs::write(
            &creds,
            format!("{{\"claudeAiOauth\":{{\"expiresAt\":{past}}}}}"),
        )
        .unwrap();
        assert_eq!(
            login_validity(&claude, home, now),
            Validity::Expired {
                expires_at_ms: past
            }
        );

        // Present, but stating no expiry.
        std::fs::write(&creds, b"{\"token\":\"abc\"}").unwrap();
        assert_eq!(login_validity(&claude, home, now), Validity::Unknown);

        temp.close().unwrap();
    }
}
