//! Materialize a profile's config overlay into an ephemeral run dir, and
//! garbage-collect stale run dirs.
//!
//! A run's config dir is a throwaway *clone* of what the run needs: the harness
//! provisioner writes `am`-managed config (`mcp.json`, `settings.json`, skills)
//! into it, then [`materialize`] layers a profile's `base/<harness>/` config
//! overlay on top — settings fragments, memory, extra skills — **without**
//! overwriting anything `am` already wrote. Overlay files are symlinked back to
//! the profile's `base/` when the platform allows (the overlay is read-only
//! config the run never mutates), falling back to a copy. Because only
//! individual *files* are linked (never directories), the runner's
//! `remove_dir_all` on exit unlinks the symlink entry and never reaches into the
//! profile's `base/`. See `_docs/target/profiles.md` §9.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context;

use crate::Result;

/// Materialize profile config-overlay `bases` (ordered **root → leaf**) into the
/// provisioned config `dir`.
///
/// Each base's files are placed into `dir` at the same relative path, but **only
/// when the destination does not already exist**. Processing runs **leaf → root**
/// so a leaf layer wins over a root layer, and any `am`-managed file the harness
/// provisioner already wrote is never clobbered — the overlay is strictly
/// additive. No-op when `bases` is empty (the common, no-profile case).
pub fn materialize(dir: &Path, bases: &[PathBuf]) -> Result<()> {
    // Leaf wins: process the highest-precedence (last) layer first, and never
    // overwrite an already-present path.
    for base in bases.iter().rev() {
        if !base.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(base).min_depth(1) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(base)?;
            let dest = dir.join(rel);
            if dest.exists() {
                continue;
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            link_or_copy(entry.path(), &dest).with_context(|| {
                format!(
                    "materializing overlay {} -> {}",
                    entry.path().display(),
                    dest.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Symlink `src` → `dst`; fall back to a copy on Windows or if the symlink can't
/// be created (e.g. no privilege). Links a single file, never a directory.
fn link_or_copy(src: &Path, dst: &Path) -> Result<()> {
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
    std::fs::copy(src, dst)?;
    Ok(())
}

/// Default run-dir time-to-live before the GC sweep removes it.
const DEFAULT_TTL_DAYS: u64 = 7;

/// Remove ephemeral run dirs under `runs_root` whose mtime is older than `ttl`.
///
/// Best-effort: unreadable or undeletable entries are skipped; symlinked base
/// files inside a run dir are unlinked, never followed. Returns how many run
/// dirs were removed. Pure (takes the TTL explicitly) so it is deterministically
/// testable; the env-driven entry point is [`sweep_old_runs`].
pub fn sweep_runs(runs_root: &Path, ttl: Duration) -> Result<usize> {
    if !runs_root.is_dir() {
        return Ok(0);
    }
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in std::fs::read_dir(runs_root)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .map(|age| age > ttl)
            .unwrap_or(false);
        if too_old && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Sweep stale run dirs using the TTL from `AM_RUNS_TTL_DAYS` (default
/// [`DEFAULT_TTL_DAYS`]). A TTL of `0` disables the sweep, so a run dir is never
/// removed out from under a concurrent run. Best-effort — callers ignore errors.
pub fn sweep_old_runs(runs_root: &Path) -> Result<usize> {
    let ttl_days = std::env::var("AM_RUNS_TTL_DAYS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TTL_DAYS);
    if ttl_days == 0 {
        return Ok(0);
    }
    sweep_runs(runs_root, Duration::from_secs(ttl_days * 24 * 60 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialize_places_overlay_files_without_clobbering_existing() {
        let base = tempfile::TempDir::new().unwrap();
        let dir = tempfile::TempDir::new().unwrap();

        // Overlay carries a new file and one that collides with an am-managed file.
        std::fs::write(base.path().join("MEMORY.md"), "from overlay").unwrap();
        std::fs::create_dir_all(base.path().join("skills/x")).unwrap();
        std::fs::write(base.path().join("skills/x/SKILL.md"), "overlay skill").unwrap();
        std::fs::write(base.path().join("settings.json"), "OVERLAY").unwrap();

        // am already wrote settings.json — must not be clobbered.
        std::fs::write(dir.path().join("settings.json"), "AM-MANAGED").unwrap();

        materialize(dir.path(), &[base.path().to_path_buf()]).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("MEMORY.md")).unwrap(),
            "from overlay"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("skills/x/SKILL.md")).unwrap(),
            "overlay skill"
        );
        // am-managed file preserved.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            "AM-MANAGED"
        );
    }

    #[test]
    fn materialize_leaf_layer_wins_over_root() {
        let root = tempfile::TempDir::new().unwrap();
        let leaf = tempfile::TempDir::new().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(root.path().join("f.txt"), "root").unwrap();
        std::fs::write(leaf.path().join("f.txt"), "leaf").unwrap();

        // bases ordered root -> leaf.
        materialize(
            dir.path(),
            &[root.path().to_path_buf(), leaf.path().to_path_buf()],
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "leaf"
        );
    }

    #[test]
    fn materialize_empty_bases_is_a_noop() {
        let dir = tempfile::TempDir::new().unwrap();
        materialize(dir.path(), &[]).unwrap();
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn sweep_removes_dirs_older_than_ttl_and_keeps_fresh_ones() {
        let runs = tempfile::TempDir::new().unwrap();
        let old = runs.path().join("old-run");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("marker"), "x").unwrap();

        // ttl = 0 => any positive age is "too old"; the dir is swept.
        let removed = sweep_runs(runs.path(), Duration::ZERO).unwrap();
        assert_eq!(removed, 1);
        assert!(!old.exists());

        // A fresh dir with a large ttl is kept.
        let keep = runs.path().join("keep-run");
        std::fs::create_dir_all(&keep).unwrap();
        let removed = sweep_runs(runs.path(), Duration::from_secs(3600)).unwrap();
        assert_eq!(removed, 0);
        assert!(keep.exists());
    }

    #[test]
    fn sweep_missing_root_is_zero() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert_eq!(sweep_runs(&missing, Duration::ZERO).unwrap(), 0);
    }
}
