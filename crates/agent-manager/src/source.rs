//! `Source` — the content seam every persistent store hands to the provisioner.
//!
//! A run's config dir is always a real on-disk directory (the harness is a real
//! subprocess that reads real files), but *where the content comes from* is
//! abstracted. A filesystem-backed store yields [`Source::Dir`] (copy or symlink
//! from an existing directory — the zero-cost path); a database- or
//! memory-backed store yields [`Source::Files`] (relative path → bytes). Both
//! materialize into the run dir the same way, so the provisioner never needs to
//! know which kind of store produced the content. This is what lets an embedder
//! back skills, credentials, and profile bases with a database instead of the
//! filesystem — see `_docs/am-as-library.md`.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

/// How [`Source::materialize`] places a [`Source::Dir`] source's files.
///
/// Ignored for [`Source::Files`] (in-memory bytes can only ever be written,
/// never symlinked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMode {
    /// Always copy. Used for credentials and anything the harness rewrites in
    /// place inside the ephemeral run dir.
    Copy,
    /// Symlink each file when the platform allows, else copy. Used for
    /// read-only overlay config the run never mutates, so cleanup's
    /// `remove_dir_all` unlinks the entry without reaching into the base.
    LinkElseCopy,
}

/// Where a store's files come from when materialized into a run dir.
///
/// The single content abstraction that replaces the raw `PathBuf`s stores used
/// to hand out (`SkillEntry.path`, an account's login `home`, a profile's
/// `base/` dir). A filesystem store produces [`Source::Dir`]; any other backend
/// produces [`Source::Files`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Source {
    /// An existing directory on disk; its files are copied or symlinked into
    /// the destination (the filesystem stores' zero-cost path).
    Dir(PathBuf),
    /// In-memory content as `(relative path, bytes)` pairs (database- or
    /// memory-backed stores).
    Files(Vec<(PathBuf, Vec<u8>)>),
}

impl Source {
    /// Materialize every file of this source into `dest`, creating parent dirs.
    ///
    /// When `clobber` is false an existing destination file is left untouched
    /// (the additive/leaf-wins semantics the overlay and template layers rely
    /// on); when true the destination is overwritten (a freshly-created skill
    /// dir, say). `mode` only affects [`Source::Dir`] — in-memory
    /// [`Source::Files`] are always written as real files.
    pub fn materialize(&self, dest: &Path, mode: LinkMode, clobber: bool) -> Result<()> {
        match self {
            Source::Dir(dir) => {
                if !dir.exists() {
                    anyhow::bail!("source path does not exist: {}", dir.display());
                }
                for entry in walkdir::WalkDir::new(dir).min_depth(1) {
                    let entry = entry?;
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let rel = entry.path().strip_prefix(dir)?;
                    place(dest, rel, mode, clobber, |target| {
                        link_or_copy(entry.path(), target, mode)
                    })?;
                }
            }
            Source::Files(files) => {
                for (rel, bytes) in files {
                    place(dest, rel, LinkMode::Copy, clobber, |target| {
                        std::fs::write(target, bytes)
                            .with_context(|| format!("writing {}", target.display()))
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Read one file's bytes by its path relative to this source, if present.
    ///
    /// Used by credential seeding, which copies specific `src → dst` files
    /// (never the whole source) and skips any that are absent.
    pub fn read(&self, rel: &Path) -> Result<Option<Vec<u8>>> {
        match self {
            Source::Dir(dir) => {
                let p = dir.join(rel);
                if !p.is_file() {
                    return Ok(None);
                }
                Ok(Some(
                    std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?,
                ))
            }
            Source::Files(files) => Ok(files
                .iter()
                .find(|(r, _)| r == rel)
                .map(|(_, b)| b.clone())),
        }
    }
}

/// Compute `dest/rel`, honor the `clobber` guard, create parent dirs, then run
/// `write` to place the file.
fn place(
    dest: &Path,
    rel: &Path,
    _mode: LinkMode,
    clobber: bool,
    write: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let target = dest.join(rel);
    if !clobber && target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write(&target)
}

/// Place a single file per `mode`: [`LinkMode::LinkElseCopy`] symlinks when it
/// can and falls back to a copy; [`LinkMode::Copy`] always copies.
fn link_or_copy(src: &Path, dst: &Path, mode: LinkMode) -> Result<()> {
    if mode == LinkMode::LinkElseCopy {
        #[cfg(unix)]
        {
            if std::os::unix::fs::symlink(src, dst).is_ok() {
                return Ok(());
            }
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(src, dst).is_ok() {
                return Ok(());
            }
        }
    }
    std::fs::copy(src, dst)
        .with_context(|| format!("copying {} -> {}", src.display(), dst.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_source_materializes_and_respects_clobber() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(src.path().join("a")).unwrap();
        std::fs::write(src.path().join("a/f.txt"), "src").unwrap();
        std::fs::write(dst.path().join("a-exists"), "existing").unwrap();

        let source = Source::Dir(src.path().to_path_buf());
        // clobber=false must not overwrite an existing file.
        std::fs::create_dir_all(dst.path().join("a")).unwrap();
        std::fs::write(dst.path().join("a/f.txt"), "KEEP").unwrap();
        source
            .materialize(dst.path(), LinkMode::Copy, false)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dst.path().join("a/f.txt")).unwrap(),
            "KEEP"
        );

        // clobber=true overwrites.
        source.materialize(dst.path(), LinkMode::Copy, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(dst.path().join("a/f.txt")).unwrap(),
            "src"
        );
    }

    #[test]
    fn files_source_writes_bytes() {
        let dst = tempfile::TempDir::new().unwrap();
        let source = Source::Files(vec![
            (PathBuf::from(".credentials.json"), b"{}".to_vec()),
            (PathBuf::from("nested/x"), b"hi".to_vec()),
        ]);
        source.materialize(dst.path(), LinkMode::Copy, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(dst.path().join(".credentials.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(dst.path().join("nested/x")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn read_finds_files_in_both_variants() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("c.json"), "DIR").unwrap();
        let d = Source::Dir(dir.path().to_path_buf());
        assert_eq!(d.read(Path::new("c.json")).unwrap().as_deref(), Some(&b"DIR"[..]));
        assert!(d.read(Path::new("missing")).unwrap().is_none());

        let f = Source::Files(vec![(PathBuf::from("c.json"), b"MEM".to_vec())]);
        assert_eq!(f.read(Path::new("c.json")).unwrap().as_deref(), Some(&b"MEM"[..]));
        assert!(f.read(Path::new("missing")).unwrap().is_none());
    }
}
