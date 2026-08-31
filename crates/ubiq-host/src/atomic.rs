//! Writing a file so that a crash never leaves half of one.
//!
//! Serialise, write a sibling temp file, fsync, rename over. The rename is atomic on every
//! filesystem Ubiq runs on, and the sibling is in the same directory so the rename never crosses a
//! filesystem boundary — which is the one thing that would make it a copy instead.
//!
//! There is no helper like this in `crates/agent-manager`; every write there is a plain
//! `fs::write`. A catalogue is worth more than a settings file, so this is new.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};

/// Enough to keep two writers in one process off each other's temp file.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path`, atomically.
///
/// A crash at any point leaves either the previous contents or the new ones, never a mixture and
/// never an empty file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed");
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    // Anything that fails from here takes the temp file with it, so a failed write leaves no
    // litter beside the real one.
    let written = (|| -> io::Result<()> {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        // The rename is only worth having if the bytes are on the disk before it happens.
        file.sync_all()?;
        Ok(())
    })();

    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    // Durability of the rename itself needs the directory synced too. A filesystem that refuses to
    // open a directory is not a reason to call the write a failure, so this one only gets logged.
    if let Ok(dir) = File::open(parent)
        && let Err(error) = dir.sync_all()
    {
        tracing::debug!("could not sync {}: {error}", parent.display());
    }

    Ok(())
}

/// Move a file aside, stamped, and answer where it went.
///
/// Losing a catalogue silently is worse than starting without one loudly, so a file that cannot be
/// parsed is preserved rather than truncated.
pub fn preserve_aside(path: &Path, now: DateTime<Utc>) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed");
    let stamp = now.format("%Y%m%dT%H%M%SZ");
    let aside = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    fs::rename(path, &aside)?;
    Ok(aside)
}
