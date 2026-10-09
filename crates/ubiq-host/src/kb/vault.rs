//! A password-protected wiki (`D206`): the key a password derives, the header that checks it, the
//! format every document is sealed in, and the keychain the password may be filed in.
//!
//! **Contents are sealed, names are not.** A protected wiki is a directory of its own,
//! `<project data>/wikis/<source id>/`, holding [`HEADER`] and the [`PAGES`] tree the source is
//! listed from. Every file under `pages/` is ciphertext; every file and folder *name* is plaintext,
//! so the tree, a rename and a link keep working without the key. Plaintext lives only in host
//! memory — nothing here writes it to disk, and the index and the watcher never reach this
//! directory (it is under the config root, and what they would find there is ciphertext anyway).
//!
//! **The key.** Argon2id over the password, with a random 16-byte salt and the cost parameters
//! written into the header, so a wiki created under one set of costs still opens after the
//! defaults change. The header also holds a *verifier* — a known constant sealed under the key —
//! so a wrong password is told apart from a damaged document, and an 8-byte random key id that
//! every document carries, so a document sealed under another key is named as such.
//!
//! **A document** is `UBQW`, a format byte, the key id, a 24-byte random nonce, and the
//! XChaCha20-Poly1305 ciphertext with its tag. The first 13 bytes are the associated data, so the
//! format byte and the key id cannot be swapped under a ciphertext. A fresh nonce on every write;
//! XChaCha's 192-bit nonce is what makes a random one safe at any count. An empty (0-byte) file is
//! read as an empty document — nothing to hide in it — and anything else without the magic is
//! refused rather than shown.
//!
//! **Changing the password** re-encrypts every document, atomically per file. It is resumable:
//! the header is first rewritten naming the new key *and* the old key sealed under the new one;
//! then each document still under the old key id is re-sealed; then the header drops the old key.
//! A crash in between leaves a header the new password opens, from which [`open`] finishes the
//! job. A forgotten password is unrecoverable — by design, there is no second way in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Generate, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use ubiq_proto::ids::KbSourceId;
use zeroize::Zeroizing;

use crate::atomic::write_atomic;

/// The header's file name, beside [`PAGES`].
pub const HEADER: &str = "vault.toml";
/// The directory the source's documents are under — what [`crate::kb::Kb::base_path`] answers.
pub const PAGES: &str = "pages";

/// The header format this Ubiq writes and reads.
const HEADER_VERSION: u32 = 1;
/// The leading bytes of every sealed document.
const MAGIC: &[u8; 4] = b"UBQW";
/// The document format this Ubiq writes and reads.
const FORMAT: u8 = 1;
const KEY_ID_LEN: usize = 8;
const NONCE_LEN: usize = 24;
/// Magic, format, key id — the associated data.
const PREFIX_LEN: usize = 4 + 1 + KEY_ID_LEN;
const SALT_LEN: usize = 16;
/// What the verifier seals. Its content is not secret; opening it is the proof.
const VERIFIER: &[u8] = b"ubiq-wiki-verifier:v1";

/// Argon2id's cost parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory, in KiB.
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    /// What a new wiki is created with: 64 MiB, three passes, one lane — above OWASP's floor (and
    /// the `argon2` crate's 19 MiB default), since a wiki's password is typed rarely. An existing
    /// wiki keeps the parameters its header names.
    fn default() -> Self {
        Self {
            m_cost: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

impl KdfParams {
    /// The cheapest parameters Argon2 accepts. For tests only: a wiki created with these is as
    /// readable as any other, and as weak as its password.
    pub fn insecure_fast() -> Self {
        Self {
            m_cost: Params::MIN_M_COST.max(8),
            t_cost: 1,
            p_cost: 1,
        }
    }
}

/// One derived key and the id the documents sealed under it carry. Wiped on drop.
pub struct Key {
    id: [u8; KEY_ID_LEN],
    bytes: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(***)")
    }
}

impl Key {
    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new_from_slice(self.bytes.as_ref()).expect("the key is 32 bytes")
    }

    /// Seal one document's plaintext.
    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        let mut out = Vec::with_capacity(PREFIX_LEN + NONCE_LEN + plain.len() + 16);
        out.extend_from_slice(MAGIC);
        out.push(FORMAT);
        out.extend_from_slice(&self.id);
        let nonce = XNonce::generate();
        let sealed = self
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: plain,
                    aad: &out[..PREFIX_LEN],
                },
            )
            .map_err(|_| VaultError::Corrupt("the document could not be sealed".into()))?;
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Open one document. An empty file is an empty document.
    pub fn open(&self, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        if sealed.is_empty() {
            return Ok(Zeroizing::new(Vec::new()));
        }
        let id = key_id_of(sealed)?;
        if id != self.id {
            return Err(VaultError::OtherKey);
        }
        let nonce = XNonce::try_from(&sealed[PREFIX_LEN..PREFIX_LEN + NONCE_LEN])
            .map_err(|_| VaultError::Corrupt("the document does not open: it is damaged".into()))?;
        self.cipher()
            .decrypt(
                &nonce,
                Payload {
                    msg: &sealed[PREFIX_LEN + NONCE_LEN..],
                    aad: &sealed[..PREFIX_LEN],
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| VaultError::Corrupt("the document does not open: it is damaged".into()))
    }
}

/// The key id a sealed document carries, after checking it is one.
fn key_id_of(sealed: &[u8]) -> Result<[u8; KEY_ID_LEN], VaultError> {
    if sealed.len() < PREFIX_LEN + NONCE_LEN + 16 || &sealed[..4] != MAGIC {
        return Err(VaultError::Corrupt(
            "this document is not sealed, so a protected wiki will not show it".into(),
        ));
    }
    if sealed[4] != FORMAT {
        return Err(VaultError::Corrupt(format!(
            "this document is in format {}, and this Ubiq reads {FORMAT}",
            sealed[4]
        )));
    }
    let mut id = [0u8; KEY_ID_LEN];
    id.copy_from_slice(&sealed[5..PREFIX_LEN]);
    Ok(id)
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("the password is wrong")]
    WrongPassword,
    #[error("this wiki has no password yet")]
    NotCreated,
    #[error("this wiki already has a password")]
    AlreadyCreated,
    #[error("this wiki's header is missing; its pages cannot be opened")]
    HeaderMissing,
    #[error("a password cannot be empty")]
    EmptyPassword,
    #[error("this document was sealed under another password")]
    OtherKey,
    #[error("{0}")]
    Corrupt(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Header {
    version: u32,
    key: KeyRecord,
    /// Present only while a password change is in flight: the old key, sealed under the new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<PreviousKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct KeyRecord {
    id: String,
    kdf: String,
    #[serde(flatten)]
    params: KdfParams,
    salt: String,
    verifier: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PreviousKey {
    id: String,
    sealed: String,
}

/// Whether a wiki directory has had a password set.
pub fn is_created(dir: &Path) -> bool {
    dir.join(HEADER).is_file()
}

/// Whether a wiki has sealed pages but no header — the header deleted or lost. Setting a password
/// then would write a new key id and orphan every page, so [`create`] refuses it.
pub fn is_orphaned(dir: &Path) -> bool {
    !is_created(dir)
        && documents(&dir.join(PAGES)).is_ok_and(|files| {
            files
                .iter()
                .any(|file| std::fs::metadata(file).is_ok_and(|meta| meta.len() > 0))
        })
}

/// Set a new wiki's password. Refused when one is already set, and when the pages a lost header
/// sealed are still there.
pub fn create(dir: &Path, password: &str, params: KdfParams) -> Result<Key, VaultError> {
    if password.is_empty() {
        return Err(VaultError::EmptyPassword);
    }
    if is_created(dir) {
        return Err(VaultError::AlreadyCreated);
    }
    if is_orphaned(dir) {
        return Err(VaultError::HeaderMissing);
    }
    std::fs::create_dir_all(dir.join(PAGES))?;
    let (key, record) = derive_new(password, params)?;
    write_header(
        dir,
        &Header {
            version: HEADER_VERSION,
            key: record,
            previous: None,
        },
    )?;
    Ok(key)
}

/// Open a wiki with its password, finishing a password change a crash interrupted.
pub fn open(dir: &Path, password: &str) -> Result<Key, VaultError> {
    let header = read_header(dir)?;
    let key = derive(password, &header.key)?;
    if let Some(previous) = &header.previous {
        let old = unseal_previous(&key, previous)?;
        reseal_all(&dir.join(PAGES), &old, &key)?;
        write_header(
            dir,
            &Header {
                previous: None,
                ..header.clone()
            },
        )?;
    }
    Ok(key)
}

/// Replace the password and re-encrypt every document under the new key.
///
/// Every document is opened under the old key first, before anything is written: a damaged one
/// refuses the change while the wiki is still whole under one password.
pub fn change(
    dir: &Path,
    old_password: &str,
    new_password: &str,
    params: KdfParams,
) -> Result<Key, VaultError> {
    if new_password.is_empty() {
        return Err(VaultError::EmptyPassword);
    }
    let old = open(dir, old_password)?;
    let pages = dir.join(PAGES);
    for file in documents(&pages)? {
        old.open(&std::fs::read(&file)?)?;
    }

    let (new, record) = derive_new(new_password, params)?;
    let sealed_old = new.seal(old.bytes.as_ref())?;
    write_header(
        dir,
        &Header {
            version: HEADER_VERSION,
            key: record.clone(),
            previous: Some(PreviousKey {
                id: hex(&old.id),
                sealed: hex(&sealed_old),
            }),
        },
    )?;
    reseal_all(&pages, &old, &new)?;
    write_header(
        dir,
        &Header {
            version: HEADER_VERSION,
            key: record,
            previous: None,
        },
    )?;
    Ok(new)
}

/// Re-seal every document still under `from` with `to`, one atomic write each.
fn reseal_all(pages: &Path, from: &Key, to: &Key) -> Result<(), VaultError> {
    for file in documents(pages)? {
        let sealed = std::fs::read(&file)?;
        if sealed.is_empty() || key_id_of(&sealed)? == to.id {
            continue;
        }
        let plain = from.open(&sealed)?;
        write_atomic(&file, &to.seal(&plain)?)?;
    }
    Ok(())
}

/// Every regular file under `pages`, recursively. Symlinks are not followed and not counted:
/// nothing Ubiq writes into a wiki is one.
fn documents(pages: &Path) -> Result<Vec<PathBuf>, VaultError> {
    let mut out = Vec::new();
    let mut stack = vec![pages.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry.file_name();
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() && !is_atomic_temp(&name.to_string_lossy()) {
                out.push(entry.path());
            }
        }
    }
    Ok(out)
}

/// An atomic write's own temp file, left by a crash — exactly `.{name}.{pid}.{n}.tmp`, the name
/// [`write_atomic`] gives it. Anything else, `.draft.tmp` included, is a document.
fn is_atomic_temp(name: &str) -> bool {
    let Some(inner) = name
        .strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    let mut parts = inner.rsplitn(3, '.');
    let (Some(n), Some(pid), Some(base)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    !base.is_empty() && digits(pid) && digits(n)
}

fn derive_new(password: &str, params: KdfParams) -> Result<(Key, KeyRecord), VaultError> {
    let salt = <[u8; SALT_LEN]>::generate();
    let id = <[u8; KEY_ID_LEN]>::generate();
    let bytes = stretch(password, &salt, params)?;
    let key = Key { id, bytes };
    let verifier = key.seal(VERIFIER)?;
    let record = KeyRecord {
        id: hex(&id),
        kdf: "argon2id".to_string(),
        params,
        salt: hex(&salt),
        verifier: hex(&verifier),
    };
    Ok((key, record))
}

/// The key a password derives against a header's record, checked against its verifier.
fn derive(password: &str, record: &KeyRecord) -> Result<Key, VaultError> {
    if record.kdf != "argon2id" {
        return Err(VaultError::Corrupt(format!(
            "this wiki's key is derived with {}, which this Ubiq does not know",
            record.kdf
        )));
    }
    let salt = unhex(&record.salt)?;
    let id: [u8; KEY_ID_LEN] = unhex(&record.id)?
        .try_into()
        .map_err(|_| VaultError::Corrupt("the header's key id is malformed".into()))?;
    let key = Key {
        id,
        bytes: stretch(password, &salt, record.params)?,
    };
    match key.open(&unhex(&record.verifier)?) {
        Ok(plain) if plain.as_slice() == VERIFIER => Ok(key),
        _ => Err(VaultError::WrongPassword),
    }
}

fn stretch(
    password: &str,
    salt: &[u8],
    params: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let params = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32))
        .map_err(|error| VaultError::Corrupt(format!("the key parameters: {error}")))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, out.as_mut())
        .map_err(|error| VaultError::Corrupt(format!("the key could not be derived: {error}")))?;
    Ok(out)
}

fn unseal_previous(current: &Key, previous: &PreviousKey) -> Result<Key, VaultError> {
    let plain = current.open(&unhex(&previous.sealed)?)?;
    let bytes: [u8; 32] = plain
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Corrupt("the previous key is malformed".into()))?;
    let id: [u8; KEY_ID_LEN] = unhex(&previous.id)?
        .try_into()
        .map_err(|_| VaultError::Corrupt("the previous key id is malformed".into()))?;
    Ok(Key {
        id,
        bytes: Zeroizing::new(bytes),
    })
}

fn read_header(dir: &Path) -> Result<Header, VaultError> {
    let raw = match std::fs::read_to_string(dir.join(HEADER)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(VaultError::NotCreated);
        }
        Err(error) => return Err(error.into()),
    };
    let header: Header = toml::from_str(&raw)
        .map_err(|error| VaultError::Corrupt(format!("the wiki's header: {error}")))?;
    if header.version > HEADER_VERSION {
        return Err(VaultError::Corrupt(format!(
            "this wiki's header is version {}, and this Ubiq reads {HEADER_VERSION}",
            header.version
        )));
    }
    Ok(header)
}

fn write_header(dir: &Path, header: &Header) -> Result<(), VaultError> {
    let body = toml::to_string_pretty(header).map_err(std::io::Error::other)?;
    write_atomic(&dir.join(HEADER), body.as_bytes())?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>, VaultError> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err(VaultError::Corrupt("the wiki's header is malformed".into()));
    }
    (0..text.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&text[at..at + 2], 16)
                .map_err(|_| VaultError::Corrupt("the wiki's header is malformed".into()))
        })
        .collect()
}

// ── the keychain ────────────────────────────────────────────────────

/// Where a protected wiki's password may be filed so the next run opens it without asking. A trait
/// so a test never touches the real keychain. The password is the material; nothing else holds it.
pub trait WikiKeychain: Send + Sync {
    fn password(&self, source: KbSourceId) -> Option<Zeroizing<String>>;
    fn remember(&self, source: KbSourceId, password: &str) -> Result<(), String>;
    fn forget(&self, source: KbSourceId) -> Result<(), String>;
}

/// No keychain at all: nothing is filed, and every run asks.
pub struct NoKeychain;

impl WikiKeychain for NoKeychain {
    fn password(&self, _source: KbSourceId) -> Option<Zeroizing<String>> {
        None
    }

    fn remember(&self, _source: KbSourceId, _password: &str) -> Result<(), String> {
        Err("this build has no keychain".to_string())
    }

    fn forget(&self, _source: KbSourceId) -> Result<(), String> {
        Ok(())
    }
}

/// A keychain in memory: the stand-in for the OS one in tests.
#[derive(Default)]
pub struct MemoryKeychain {
    held: Mutex<HashMap<KbSourceId, Zeroizing<String>>>,
}

impl MemoryKeychain {
    pub fn holds(&self, source: KbSourceId) -> bool {
        self.lock().contains_key(&source)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<KbSourceId, Zeroizing<String>>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl WikiKeychain for MemoryKeychain {
    fn password(&self, source: KbSourceId) -> Option<Zeroizing<String>> {
        self.lock().get(&source).cloned()
    }

    fn remember(&self, source: KbSourceId, password: &str) -> Result<(), String> {
        self.lock()
            .insert(source, Zeroizing::new(password.to_string()));
        Ok(())
    }

    fn forget(&self, source: KbSourceId) -> Result<(), String> {
        self.lock().remove(&source);
        Ok(())
    }
}

/// The OS keychain, through the connector family's store — the one place Ubiq files secrets.
#[cfg(feature = "listener")]
impl WikiKeychain for crate::connectors::store::Store {
    fn password(&self, source: KbSourceId) -> Option<Zeroizing<String>> {
        self.wiki_password(source).map(Zeroizing::new)
    }

    fn remember(&self, source: KbSourceId, password: &str) -> Result<(), String> {
        self.set_wiki_password(source, password)
    }

    fn forget(&self, source: KbSourceId) -> Result<(), String> {
        self.clear_wiki_password(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams::insecure_fast()
    }

    #[test]
    fn a_document_round_trips_and_the_disk_holds_no_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let key = create(dir.path(), "correct horse", fast()).unwrap();
        let sealed = key.seal(b"the needle in the haystack").unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"needle"));
        assert_eq!(&sealed[..4], MAGIC);
        assert_eq!(
            key.open(&sealed).unwrap().as_slice(),
            b"the needle in the haystack"
        );

        // Two seals of the same text differ: a fresh nonce each time.
        assert_ne!(key.seal(b"same").unwrap(), key.seal(b"same").unwrap());

        let reopened = open(dir.path(), "correct horse").unwrap();
        assert_eq!(
            reopened.open(&sealed).unwrap().as_slice(),
            b"the needle in the haystack"
        );
    }

    #[test]
    fn a_wrong_password_is_named_as_such() {
        let dir = tempfile::tempdir().unwrap();
        create(dir.path(), "right", fast()).unwrap();
        assert!(matches!(
            open(dir.path(), "wrong"),
            Err(VaultError::WrongPassword)
        ));
        assert!(matches!(
            open(tempfile::tempdir().unwrap().path(), "x"),
            Err(VaultError::NotCreated)
        ));
        assert!(matches!(
            create(dir.path(), "again", fast()),
            Err(VaultError::AlreadyCreated)
        ));
    }

    #[test]
    fn a_tampered_document_or_a_plaintext_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let key = create(dir.path(), "pw", fast()).unwrap();
        let mut sealed = key.seal(b"text").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(matches!(key.open(&sealed), Err(VaultError::Corrupt(_))));
        assert!(matches!(
            key.open(b"# plain markdown"),
            Err(VaultError::Corrupt(_))
        ));
        assert!(key.open(b"").unwrap().is_empty());
    }

    #[test]
    fn changing_the_password_reseals_every_document() {
        let dir = tempfile::tempdir().unwrap();
        let old = create(dir.path(), "old", fast()).unwrap();
        let pages = dir.path().join(PAGES);
        std::fs::create_dir_all(pages.join("sub")).unwrap();
        std::fs::write(pages.join("a.md"), old.seal(b"alpha").unwrap()).unwrap();
        std::fs::write(pages.join("sub/b.md"), old.seal(b"beta").unwrap()).unwrap();

        let new = change(dir.path(), "old", "new", fast()).unwrap();
        assert!(matches!(
            open(dir.path(), "old"),
            Err(VaultError::WrongPassword)
        ));
        let reopened = open(dir.path(), "new").unwrap();
        for (file, text) in [("a.md", "alpha"), ("sub/b.md", "beta")] {
            let sealed = std::fs::read(pages.join(file)).unwrap();
            assert!(matches!(old.open(&sealed), Err(VaultError::OtherKey)));
            assert_eq!(new.open(&sealed).unwrap().as_slice(), text.as_bytes());
            assert_eq!(reopened.open(&sealed).unwrap().as_slice(), text.as_bytes());
        }
        assert!(
            !std::fs::read_to_string(dir.path().join(HEADER))
                .unwrap()
                .contains("previous")
        );
    }

    #[test]
    fn a_wrong_old_password_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let key = create(dir.path(), "old", fast()).unwrap();
        let file = dir.path().join(PAGES).join("a.md");
        std::fs::write(&file, key.seal(b"alpha").unwrap()).unwrap();
        let before = std::fs::read(&file).unwrap();
        assert!(matches!(
            change(dir.path(), "nope", "new", fast()),
            Err(VaultError::WrongPassword)
        ));
        assert_eq!(std::fs::read(&file).unwrap(), before);
        open(dir.path(), "old").unwrap();
    }

    #[test]
    fn an_interrupted_change_is_finished_by_the_next_open() {
        let dir = tempfile::tempdir().unwrap();
        let old = create(dir.path(), "old", fast()).unwrap();
        let pages = dir.path().join(PAGES);
        std::fs::write(pages.join("a.md"), old.seal(b"alpha").unwrap()).unwrap();
        std::fs::write(pages.join("b.md"), old.seal(b"beta").unwrap()).unwrap();

        // The first half of `change` by hand: the new header naming the old key, and one document
        // re-sealed — then a "crash".
        let (new, record) = derive_new("new", fast()).unwrap();
        write_header(
            dir.path(),
            &Header {
                version: HEADER_VERSION,
                key: record,
                previous: Some(PreviousKey {
                    id: hex(&old.id),
                    sealed: hex(&new.seal(old.bytes.as_ref()).unwrap()),
                }),
            },
        )
        .unwrap();
        let a = old
            .open(&std::fs::read(pages.join("a.md")).unwrap())
            .unwrap();
        write_atomic(&pages.join("a.md"), &new.seal(&a).unwrap()).unwrap();

        let reopened = open(dir.path(), "new").unwrap();
        for (file, text) in [("a.md", "alpha"), ("b.md", "beta")] {
            let sealed = std::fs::read(pages.join(file)).unwrap();
            assert_eq!(reopened.open(&sealed).unwrap().as_slice(), text.as_bytes());
        }
        assert!(
            !std::fs::read_to_string(dir.path().join(HEADER))
                .unwrap()
                .contains("previous")
        );
    }

    #[test]
    fn only_the_exact_atomic_temp_name_is_skipped() {
        assert!(is_atomic_temp(".a.md.1234.7.tmp"));
        assert!(is_atomic_temp(".vault.toml.1.0.tmp"));
        for name in [
            ".draft.tmp",
            ".a.md.x.7.tmp",
            ".a.md.1.tmp",
            "a.md.1.2.tmp",
            "..1.2.tmp",
        ] {
            assert!(!is_atomic_temp(name), "{name}");
        }

        let dir = tempfile::tempdir().unwrap();
        let old = create(dir.path(), "old", fast()).unwrap();
        let pages = dir.path().join(PAGES);
        std::fs::write(pages.join(".draft.tmp"), old.seal(b"draft").unwrap()).unwrap();
        std::fs::write(pages.join(".a.md.99.0.tmp"), b"crash litter").unwrap();
        let new = change(dir.path(), "old", "new", fast()).unwrap();
        let sealed = std::fs::read(pages.join(".draft.tmp")).unwrap();
        assert_eq!(new.open(&sealed).unwrap().as_slice(), b"draft");
    }

    #[test]
    fn a_lost_header_over_sealed_pages_refuses_a_new_password() {
        let dir = tempfile::tempdir().unwrap();
        let key = create(dir.path(), "pw", fast()).unwrap();
        let pages = dir.path().join(PAGES);
        std::fs::write(pages.join("a.md"), key.seal(b"alpha").unwrap()).unwrap();
        std::fs::remove_file(dir.path().join(HEADER)).unwrap();
        assert!(is_orphaned(dir.path()));
        let refused = create(dir.path(), "new", fast()).unwrap_err();
        assert!(matches!(refused, VaultError::HeaderMissing));
        assert_eq!(
            refused.to_string(),
            "this wiki's header is missing; its pages cannot be opened"
        );
        assert!(!is_created(dir.path()));

        // Empty pages hide nothing: a wiki holding only those may still be created.
        std::fs::write(pages.join("a.md"), b"").unwrap();
        assert!(!is_orphaned(dir.path()));
        create(dir.path(), "new", fast()).unwrap();
    }

    #[test]
    fn a_new_wiki_defaults_to_64_mib_three_passes() {
        assert_eq!(
            KdfParams::default(),
            KdfParams {
                m_cost: 65536,
                t_cost: 3,
                p_cost: 1
            }
        );
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(
            unhex(&hex(&[0, 1, 0xab, 0xff])).unwrap(),
            vec![0, 1, 0xab, 0xff]
        );
        assert!(unhex("abc").is_err());
    }
}
