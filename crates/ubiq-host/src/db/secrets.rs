//! Saved database passwords: sealed on disk, opened only in the host.
//!
//! Each password is encrypted with AES-256-GCM (`ring`, a fresh 96-bit nonce per write) under one
//! key per install, which lives in the OS keychain through [`KeySource`] — in production the
//! connector family's [`Store`], in tests a [`MemoryKeys`]. The associated data is
//! `ubiq-db:v1:<project id>:<connection id>`, so a ciphertext copied onto another entry fails to
//! open. The ciphertext sits in `<config root>/projects/<ulid>/local/db-secrets.toml`, whatever the
//! project's storage mode: the key that opens it is this machine's, so the file never goes where a
//! `git add -f` could reach it.
//!
//! **A missing password is a prompt, never an error.** The key gone (a reset keychain) or a
//! ciphertext that does not open (a copied config root) reads [`PasswordState::Missing`]. With the
//! keychain unusable nothing is written, and a typed password is held in memory for the run
//! ([`PasswordState::Session`]).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use ubiq_proto::db::{DbKeystore, PasswordState};
use ubiq_proto::ids::{DbConnId, ProjectId};

use crate::atomic::write_atomic;
use crate::connectors::store::Store;
use crate::store::project_dir::ProjectData;

/// The ciphertext file, inside a project's `local/`.
pub const SECRETS_FILE: &str = "db-secrets.toml";

/// The format this Ubiq writes and understands.
pub const SECRETS_VERSION: u32 = 1;

/// Where a project's sealed passwords are. Always under the config root, never in `.ubiq/`.
pub fn path(root: &Path, project: ProjectId) -> PathBuf {
    ProjectData::under_config(root, project)
        .local()
        .join(SECRETS_FILE)
}

/// Where the install's key is kept. A trait so a test never touches the real keychain.
pub trait KeySource: Send + Sync {
    /// Whether the key can be kept at all.
    fn usable(&self) -> Result<(), String>;
    /// The filed key, base64. `Ok(None)` is nothing filed; `Err` is a store that cannot be read.
    fn load(&self) -> Result<Option<String>, String>;
    fn store(&self, key: &str) -> Result<(), String>;
}

impl KeySource for Store {
    fn usable(&self) -> Result<(), String> {
        Store::usable(self)
    }

    fn load(&self) -> Result<Option<String>, String> {
        self.db_key_value()
    }

    fn store(&self, key: &str) -> Result<(), String> {
        self.set_db_key(key)
    }
}

/// A key held in memory: the stand-in for the keychain in tests, and the way to make one unusable
/// or lose its key.
pub struct MemoryKeys {
    usable: Result<(), String>,
    key: Mutex<Option<String>>,
}

impl MemoryKeys {
    pub fn new() -> Self {
        Self {
            usable: Ok(()),
            key: Mutex::new(None),
        }
    }

    /// A keychain that does not work, for the reason given.
    pub fn unusable(why: impl Into<String>) -> Self {
        Self {
            usable: Err(why.into()),
            key: Mutex::new(None),
        }
    }

    /// A reset keychain: the filed key disappears.
    pub fn lose(&self) {
        *lock(&self.key) = None;
    }

    pub fn current(&self) -> Option<String> {
        lock(&self.key).clone()
    }
}

impl Default for MemoryKeys {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySource for MemoryKeys {
    fn usable(&self) -> Result<(), String> {
        self.usable.clone()
    }

    fn load(&self) -> Result<Option<String>, String> {
        Ok(self.current())
    }

    fn store(&self, key: &str) -> Result<(), String> {
        *lock(&self.key) = Some(key.to_string());
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the keychain: {0}")]
    Keystore(String),
    #[error("{path} is version {found}, and this Ubiq understands {SECRETS_VERSION}")]
    TooNew { path: PathBuf, found: u32 },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SecretsFile {
    version: u32,
    #[serde(default, rename = "entry")]
    entries: BTreeMap<String, Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    nonce: String,
    ciphertext: String,
}

/// The passwords of every project on this install.
pub struct Secrets {
    root: PathBuf,
    keys: Arc<dyn KeySource>,
    /// Passwords held for this run only.
    session: Mutex<HashMap<(ProjectId, DbConnId), String>>,
    /// Serialises read-modify-write of the files and the key's creation.
    write: Mutex<()>,
}

impl Secrets {
    pub fn new(root: PathBuf, keys: Arc<dyn KeySource>) -> Self {
        Self {
            root,
            keys,
            session: Mutex::new(HashMap::new()),
            write: Mutex::new(()),
        }
    }

    /// Whether this install can seal a password at all.
    pub fn keystore(&self) -> DbKeystore {
        match self.keys.usable() {
            Ok(()) => DbKeystore::Ready,
            Err(why) => DbKeystore::Unavailable(why),
        }
    }

    /// The state of each connection's password, one read of the file and one of the key.
    pub fn states(&self, project: ProjectId, ids: &[DbConnId]) -> Vec<PasswordState> {
        let file = self.read_file(project).unwrap_or_default();
        let key = if file.entries.is_empty() {
            None
        } else {
            self.read_key()
        };
        let session = lock(&self.session);
        ids.iter()
            .map(|id| {
                if session.contains_key(&(project, *id)) {
                    return PasswordState::Session;
                }
                match file.entries.get(&id.to_string()) {
                    None => PasswordState::None,
                    Some(entry) => match &key {
                        Some(key) if open(key, &aad(project, *id), entry).is_some() => {
                            PasswordState::Saved
                        }
                        _ => PasswordState::Missing,
                    },
                }
            })
            .collect()
    }

    pub fn state(&self, project: ProjectId, id: DbConnId) -> PasswordState {
        self.states(project, &[id])[0]
    }

    /// The password, for a connection about to be dialled. `None` is a prompt.
    pub fn password(&self, project: ProjectId, id: DbConnId) -> Option<String> {
        if let Some(held) = lock(&self.session).get(&(project, id)) {
            return Some(held.clone());
        }
        let file = self.read_file(project).ok()?;
        let entry = file.entries.get(&id.to_string())?;
        open(&self.read_key()?, &aad(project, id), entry)
    }

    /// Keep a password. `remember` seals it to disk under the install's key (creating the key the
    /// first time, or again if it was lost — which drops this project's entries the old key
    /// sealed); otherwise, or with the keychain unusable, it is held for the run. Returns where it
    /// ended up.
    pub fn remember(
        &self,
        project: ProjectId,
        id: DbConnId,
        password: &str,
        remember: bool,
    ) -> Result<PasswordState, SecretsError> {
        let _guard = lock(&self.write);
        if !remember || self.keys.usable().is_err() {
            lock(&self.session).insert((project, id), password.to_string());
            return Ok(PasswordState::Session);
        }

        let mut file = self.read_file(project)?;
        let (key, fresh) = match self.keys.load().map_err(SecretsError::Keystore)? {
            Some(text) if decode_key(&text).is_some() => (decode_key(&text), false),
            _ => {
                let mut bytes = [0u8; 32];
                SystemRandom::new()
                    .fill(&mut bytes)
                    .map_err(|_| SecretsError::Keystore("no source of randomness".into()))?;
                self.keys
                    .store(&STANDARD.encode(bytes))
                    .map_err(SecretsError::Keystore)?;
                (aead_key(&bytes), true)
            }
        };
        let key = key.ok_or_else(|| SecretsError::Keystore("the key is unusable".into()))?;
        if fresh {
            file.entries.clear();
        }
        file.entries
            .insert(id.to_string(), seal(&key, &aad(project, id), password)?);
        self.write_file(project, &file)?;
        lock(&self.session).remove(&(project, id));
        Ok(PasswordState::Saved)
    }

    /// Forget a connection's password everywhere: its entry in the file and the run's copy.
    pub fn forget(&self, project: ProjectId, id: DbConnId) -> Result<(), SecretsError> {
        let _guard = lock(&self.write);
        lock(&self.session).remove(&(project, id));
        let mut file = match self.read_file(project) {
            Ok(file) => file,
            // Nothing readable, nothing of ours to remove; a too-new file is not ours to edit.
            Err(_) => return Ok(()),
        };
        if file.entries.remove(&id.to_string()).is_some() {
            self.write_file(project, &file)?;
        }
        Ok(())
    }

    /// The key as a usable AEAD key, or `None` when it is not there, not readable or not ours.
    fn read_key(&self) -> Option<LessSafeKey> {
        self.keys.usable().ok()?;
        decode_key(&self.keys.load().ok()??)
    }

    fn read_file(&self, project: ProjectId) -> Result<SecretsFile, SecretsError> {
        let file = path(&self.root, project);
        let raw = match std::fs::read_to_string(&file) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SecretsFile::default());
            }
            Err(error) => return Err(error.into()),
        };
        match toml::from_str::<SecretsFile>(&raw) {
            Ok(parsed) if parsed.version > SECRETS_VERSION => Err(SecretsError::TooNew {
                path: file,
                found: parsed.version,
            }),
            Ok(parsed) => Ok(parsed),
            Err(error) => {
                // Sealed passwords cost a prompt each to recreate, not data: read as none.
                tracing::warn!("ignoring an unreadable {}: {error}", file.display());
                Ok(SecretsFile::default())
            }
        }
    }

    fn write_file(&self, project: ProjectId, file: &SecretsFile) -> Result<(), SecretsError> {
        let file = SecretsFile {
            version: SECRETS_VERSION,
            entries: file.entries.clone(),
        };
        let body = toml::to_string_pretty(&file).map_err(std::io::Error::other)?;
        write_atomic(&path(&self.root, project), body.as_bytes())?;
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The associated data: a ciphertext is good for exactly one project's one connection.
fn aad(project: ProjectId, id: DbConnId) -> String {
    format!("ubiq-db:v1:{project}:{id}")
}

fn aead_key(bytes: &[u8]) -> Option<LessSafeKey> {
    UnboundKey::new(&AES_256_GCM, bytes)
        .ok()
        .map(LessSafeKey::new)
}

fn decode_key(text: &str) -> Option<LessSafeKey> {
    aead_key(&STANDARD.decode(text.trim()).ok()?)
}

fn seal(key: &LessSafeKey, aad: &str, plain: &str) -> Result<Entry, SecretsError> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| SecretsError::Keystore("no source of randomness".into()))?;
    let mut buffer = plain.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(aad.as_bytes()),
        &mut buffer,
    )
    .map_err(|_| SecretsError::Keystore("the password could not be sealed".into()))?;
    Ok(Entry {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(buffer),
    })
}

fn open(key: &LessSafeKey, aad: &str, entry: &Entry) -> Option<String> {
    let nonce: [u8; NONCE_LEN] = STANDARD.decode(&entry.nonce).ok()?.try_into().ok()?;
    let mut buffer = STANDARD.decode(&entry.ciphertext).ok()?;
    let plain = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad.as_bytes()),
            &mut buffer,
        )
        .ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        keys: Arc<MemoryKeys>,
        secrets: Secrets,
        project: ProjectId,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("config root");
        let keys = Arc::new(MemoryKeys::new());
        let secrets = Secrets::new(dir.path().to_path_buf(), keys.clone());
        Fixture {
            _dir: dir,
            keys,
            secrets,
            project: ProjectId::generate(),
        }
    }

    #[test]
    fn a_password_round_trips_and_never_reaches_the_disk_in_the_clear() {
        let f = fixture();
        let id = DbConnId::generate();
        assert_eq!(f.secrets.state(f.project, id), PasswordState::None);

        let state = f.secrets.remember(f.project, id, "hunter2-needle", true);
        assert_eq!(state.expect("remembered"), PasswordState::Saved);
        assert_eq!(f.secrets.state(f.project, id), PasswordState::Saved);
        assert_eq!(
            f.secrets.password(f.project, id).as_deref(),
            Some("hunter2-needle")
        );

        let raw = std::fs::read_to_string(path(&f.secrets.root, f.project)).expect("file");
        assert!(
            !raw.contains("needle") && raw.contains("ciphertext"),
            "{raw}"
        );
        assert!(path(&f.secrets.root, f.project).ends_with("local/db-secrets.toml"));

        // A fresh instance over the same root and keychain reads it back.
        let again = Secrets::new(f.secrets.root.clone(), f.keys.clone());
        assert_eq!(
            again.password(f.project, id).as_deref(),
            Some("hunter2-needle")
        );
    }

    #[test]
    fn every_write_uses_a_new_nonce() {
        let f = fixture();
        let id = DbConnId::generate();
        f.secrets
            .remember(f.project, id, "same", true)
            .expect("one");
        let first = f.secrets.read_file(f.project).expect("file").entries;
        f.secrets
            .remember(f.project, id, "same", true)
            .expect("two");
        let second = f.secrets.read_file(f.project).expect("file").entries;
        assert_ne!(first, second);
    }

    #[test]
    fn a_ciphertext_copied_to_another_connection_is_refused() {
        let f = fixture();
        let (a, b) = (DbConnId::generate(), DbConnId::generate());
        f.secrets.remember(f.project, a, "for-a", true).expect("a");

        // The same bytes under another connection's name: the associated data differs.
        let mut file = f.secrets.read_file(f.project).expect("file");
        let entry = file.entries[&a.to_string()].clone();
        file.entries.insert(b.to_string(), entry.clone());
        f.secrets.write_file(f.project, &file).expect("write");
        assert_eq!(f.secrets.state(f.project, b), PasswordState::Missing);
        assert_eq!(f.secrets.password(f.project, b), None);
        assert_eq!(f.secrets.state(f.project, a), PasswordState::Saved);

        // And the same connection in another project.
        let key = f.secrets.read_key().expect("key");
        assert!(open(&key, &aad(f.project, a), &entry).is_some());
        assert!(open(&key, &aad(ProjectId::generate(), a), &entry).is_none());
        assert!(open(&key, &aad(f.project, b), &entry).is_none());
    }

    #[test]
    fn a_lost_key_reads_missing_and_a_new_save_recovers() {
        let f = fixture();
        let (a, b) = (DbConnId::generate(), DbConnId::generate());
        f.secrets.remember(f.project, a, "pw-a", true).expect("a");
        f.secrets.remember(f.project, b, "pw-b", true).expect("b");
        let old_key = f.keys.current();

        f.keys.lose();
        assert_eq!(f.secrets.state(f.project, a), PasswordState::Missing);
        assert_eq!(
            f.secrets.states(f.project, &[a, b]),
            vec![PasswordState::Missing; 2]
        );
        assert_eq!(f.secrets.password(f.project, a), None);

        // Re-entering one re-encrypts under a fresh key and drops what the old key sealed.
        f.secrets
            .remember(f.project, a, "pw-a2", true)
            .expect("again");
        assert_ne!(f.keys.current(), old_key);
        assert_eq!(f.secrets.password(f.project, a).as_deref(), Some("pw-a2"));
        assert_eq!(f.secrets.state(f.project, b), PasswordState::None);
    }

    #[test]
    fn an_unusable_keychain_writes_nothing_and_holds_the_password_for_the_run() {
        let dir = tempfile::tempdir().expect("config root");
        let secrets = Secrets::new(
            dir.path().to_path_buf(),
            Arc::new(MemoryKeys::unusable("no secret service")),
        );
        let (project, id) = (ProjectId::generate(), DbConnId::generate());
        assert_eq!(
            secrets.keystore(),
            DbKeystore::Unavailable("no secret service".into())
        );

        let state = secrets.remember(project, id, "pw", true).expect("held");
        assert_eq!(state, PasswordState::Session);
        assert_eq!(secrets.state(project, id), PasswordState::Session);
        assert_eq!(secrets.password(project, id).as_deref(), Some("pw"));
        assert!(!path(dir.path(), project).exists());
    }

    #[test]
    fn not_remembering_is_a_session_password_and_forget_clears_both() {
        let f = fixture();
        let id = DbConnId::generate();
        let held = f
            .secrets
            .remember(f.project, id, "pw", false)
            .expect("held");
        assert_eq!(held, PasswordState::Session);
        assert!(!path(&f.secrets.root, f.project).exists());

        f.secrets
            .remember(f.project, id, "pw", true)
            .expect("saved");
        f.secrets.forget(f.project, id).expect("forgotten");
        assert_eq!(f.secrets.state(f.project, id), PasswordState::None);
        assert_eq!(f.secrets.password(f.project, id), None);
    }

    #[test]
    fn a_file_from_a_newer_ubiq_is_not_overwritten() {
        let f = fixture();
        let file = path(&f.secrets.root, f.project);
        std::fs::create_dir_all(file.parent().expect("local/")).expect("dir");
        std::fs::write(&file, "version = 7\n").expect("write");
        let result = f
            .secrets
            .remember(f.project, DbConnId::generate(), "pw", true);
        assert!(matches!(result, Err(SecretsError::TooNew { found: 7, .. })));
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "version = 7\n"
        );
    }
}
