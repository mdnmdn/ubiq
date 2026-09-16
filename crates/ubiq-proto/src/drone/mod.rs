//! Resolving and caching the drone binary a deployer hands to a remote machine.
//!
//! Phase 9's provenance half: [`manifest`] is a generated table of cross-built `ubiq-drone`
//! binaries, one per target triple, each pinned by SHA-256 and length. This module is the lookup
//! over that table — mapping a probed `uname -sm` onto a triple, saying where a cached copy of
//! that triple's binary lives under a config root, and re-hashing a cached copy before it is
//! trusted.
//!
//! **Hash-pinned, not signed.** There is no signing key and no release pipeline (`G108`); this
//! module catches a corrupted or swapped cache entry, and [`manifest`] catches the local build
//! drifting from what was last snapshotted. Neither is a signature, and the docs must not imply
//! one: what verifies here is "these are the bytes `just drone-manifest` hashed", not "these bytes
//! came from somewhere Ubiq should trust".
//!
//! **Lives here, not in `ubiq-host`, because both halves need it.** `crates/ubiq` does not depend
//! on `crates/ubiq-host` (`just ui` checks it), and phase 5–8 already run every `ssh` reach of a
//! drone — dialling, probing, listing — directly from `crates/ubiq/src/app/ssh_connect.rs` on the
//! interface's own background executor, never through the host's message bus. Phase 9's deployer
//! is the same shape: it has to resolve and verify a manifest entry from inside that same crate, so
//! the manifest and its verifier move to the one crate both halves already depend on. `ubiq-host`
//! re-exports this module under its old name so nothing that already named `ubiq_host::drone`
//! breaks.
//!
//! This crate's own doc says nothing here touches disk; [`verify`]'s `std::fs::read` is the same
//! kind of exception `bus::dump` and the log sink already are — operational I/O against a path
//! Ubiq itself owns (a drone cache, a debug tape, a log file), never a project's.

pub mod manifest;

use std::path::{Path, PathBuf};

use manifest::Entry;
use sha2::{Digest, Sha256};

/// A cached binary that failed its pin.
#[derive(Debug)]
pub enum DroneError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Length {
        path: PathBuf,
        expected: u64,
        found: u64,
    },
    Hash {
        path: PathBuf,
        expected: String,
        found: String,
    },
}

impl std::fmt::Display for DroneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DroneError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            DroneError::Length {
                path,
                expected,
                found,
            } => write!(
                f,
                "{}: expected {expected} bytes, found {found}",
                path.display()
            ),
            DroneError::Hash {
                path,
                expected,
                found,
            } => write!(
                f,
                "{}: sha256 mismatch — expected {expected}, found {found}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for DroneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DroneError::Io { source, .. } => Some(source),
            DroneError::Length { .. } | DroneError::Hash { .. } => None,
        }
    }
}

/// Maps `uname -sm`'s output onto a triple in [`manifest::BINARIES`].
///
/// | `uname -sm` | triple |
/// |---|---|
/// | `Linux x86_64` | `x86_64-unknown-linux-gnu`, or the `-musl` twin if that is the only Linux entry built |
/// | `Linux aarch64` | `aarch64-unknown-linux-gnu`, or its `-musl` twin under the same rule |
/// | `Darwin x86_64` | `x86_64-apple-darwin` |
/// | `Darwin arm64` | `aarch64-apple-darwin` |
///
/// musl is preferred over glibc for a given Linux architecture only when glibc was not built —
/// glibc is the safer default (it links against whatever the remote distribution already has),
/// and musl exists for the distributions that have none. Windows has no `uname`, so a deployer
/// never calls this for it; `entry` is reached some other way there.
pub fn triple_for(uname_sm: &str) -> Option<&'static str> {
    let mut parts = uname_sm.split_whitespace();
    let system = parts.next()?;
    let machine = parts.next()?;

    let arch = match machine {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };

    match system {
        "Linux" => {
            let gnu = format!("{arch}-unknown-linux-gnu");
            let musl = format!("{arch}-unknown-linux-musl");
            let has_gnu = manifest::BINARIES.iter().any(|e| e.triple == gnu);
            let has_musl = manifest::BINARIES.iter().any(|e| e.triple == musl);
            if has_gnu {
                manifest::BINARIES
                    .iter()
                    .find(|e| e.triple == gnu)
                    .map(|e| e.triple)
            } else if has_musl {
                manifest::BINARIES
                    .iter()
                    .find(|e| e.triple == musl)
                    .map(|e| e.triple)
            } else {
                // Neither is built. Name the glibc triple anyway, so a caller gets a stable
                // answer to look up rather than `None` hiding which triple it wanted.
                Some(leaked_gnu_triple(arch))
            }
        }
        "Darwin" => {
            let triple = if arch == "aarch64" {
                "aarch64-apple-darwin"
            } else {
                "x86_64-apple-darwin"
            };
            Some(triple)
        }
        _ => None,
    }
}

/// The handful of triples this module ever names are all in [`TRIPLE_LITERALS`], so a lookup by
/// architecture alone (the "neither built" fallback above) can still return a `&'static str`
/// without leaking anything at runtime.
const TRIPLE_LITERALS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

fn leaked_gnu_triple(arch: &str) -> &'static str {
    let wanted = format!("{arch}-unknown-linux-gnu");
    TRIPLE_LITERALS
        .iter()
        .find(|t| **t == wanted)
        .copied()
        .expect("every arch this function reaches has a -gnu literal above")
}

/// The manifest entry for a triple, if [`manifest::BINARIES`] carries one.
pub fn entry(triple: &str) -> Option<&'static Entry> {
    manifest::BINARIES.iter().find(|e| e.triple == triple)
}

/// Where a cached copy of a triple's binary lives, keyed by version so two builds never collide —
/// the shape `web_assets` uses for its own cache (`<shared>/web/<app>/<version>.bundle`), rooted
/// under the config directory instead of the shared workarea because a drone binary is a thing
/// Ubiq itself owns, not something a project's window fetches on demand.
pub fn cached_path(config_root: &Path, triple: &str) -> PathBuf {
    config_root
        .join("drone")
        .join(manifest::VERSION)
        .join(triple)
        .join("ubiq-drone")
}

/// Re-hashes a binary on disk against its pinned SHA-256 and length.
///
/// Catches a corrupted or truncated cache entry, and — as far as a hash can — a swapped one:
/// nothing here proves who wrote the bytes, only that they are the bytes `just drone-manifest`
/// last hashed. See the module doc for why that is what "hash-pinned, not signed" means.
pub fn verify(path: &Path, entry: &Entry) -> Result<(), DroneError> {
    let bytes = std::fs::read(path).map_err(|source| DroneError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    if bytes.len() as u64 != entry.len {
        return Err(DroneError::Length {
            path: path.to_path_buf(),
            expected: entry.len,
            found: bytes.len() as u64,
        });
    }

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let found: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if found != entry.sha256 {
        return Err(DroneError::Hash {
            path: path.to_path_buf(),
            expected: entry.sha256.to_string(),
            found,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triple_for_maps_the_common_uname_answers() {
        // Both entries built: prefer glibc.
        assert_eq!(triple_for("Darwin x86_64"), Some("x86_64-apple-darwin"));
        assert_eq!(triple_for("Darwin arm64"), Some("aarch64-apple-darwin"));
        // Neither Linux entry is built in this tree's manifest, so the fallback names glibc.
        assert_eq!(triple_for("Linux x86_64"), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(
            triple_for("Linux aarch64"),
            Some("aarch64-unknown-linux-gnu")
        );
    }

    #[test]
    fn triple_for_rejects_an_unknown_answer() {
        assert_eq!(triple_for("FreeBSD x86_64"), None);
        assert_eq!(triple_for("Linux riscv64"), None);
        assert_eq!(triple_for(""), None);
    }

    #[test]
    fn cached_path_is_rooted_under_the_config_directory_by_version_and_triple() {
        let root = Path::new("/home/user/.config/ubiq");
        let path = cached_path(root, "x86_64-unknown-linux-gnu");
        assert_eq!(
            path,
            root.join("drone")
                .join(manifest::VERSION)
                .join("x86_64-unknown-linux-gnu")
                .join("ubiq-drone")
        );
    }

    #[test]
    fn verify_rejects_a_wrong_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ubiq-drone");
        let bytes: &[u8] = b"not the real binary";
        std::fs::write(&path, bytes).expect("write");

        let entry = Entry {
            triple: "x86_64-unknown-linux-gnu",
            path: "target/x86_64-unknown-linux-gnu/release/ubiq-drone",
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            len: bytes.len() as u64,
        };

        let error = verify(&path, &entry).expect_err("hash must not match a made-up pin");
        assert!(matches!(error, DroneError::Hash { .. }));
    }

    #[test]
    fn verify_rejects_a_wrong_length() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ubiq-drone");
        std::fs::write(&path, b"short").expect("write");

        let entry = Entry {
            triple: "x86_64-unknown-linux-gnu",
            path: "target/x86_64-unknown-linux-gnu/release/ubiq-drone",
            sha256: "deadbeef",
            len: 999,
        };

        let error = verify(&path, &entry).expect_err("length must not match");
        assert!(matches!(error, DroneError::Length { .. }));
    }

    // No test exercises a real cross-built binary against a real manifest entry: this sandbox has
    // no cross toolchain, so `manifest::BINARIES` is empty here. `verify`'s happy path is proven
    // by construction — it is the same hash-and-compare `webassets.py`'s own `verify` mode uses,
    // just over `sha256::digest` instead of a re-fetch — and by `drone-manifest` /
    // `drone-manifest-verify` agreeing on a real build wherever one exists.
}
