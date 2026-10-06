//! The process half of the Archify engine's repository-evidence verification: [`ProcessGit`] answers the
//! [`ubiq_archify::evidence::Git`] questions with the `git` executable (`git --no-replace-objects -C
//! <root>`: a pinned SHA must read the original objects, never what a local replace ref swaps in),
//! `fs::canonicalize` and physical entry identity. Nothing here touches the network or the user's
//! SSH config, and nothing reads the working tree: citations are checked against the committed blobs
//! of the pinned revision.
//!
//! The pure algorithm, its order and its messages are [`ubiq_archify::evidence::verify`]; [`verifier`] is
//! the closure `compile_with` takes, bound to one `--repo-root`.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use ubiq_archify::diag::Diagnostic;
use ubiq_archify::evidence::{self, Entry, Git, GitRun, RootError};
use ubiq_archify::model::Doc;

/// `MAX_SOURCE_BYTES`: a call whose output outgrows it is `git-unavailable` (Node's `ENOBUFS`).
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// The real `git`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessGit;

/// The verifier of one repository root: `compile_with(.., Some(&verifier(root)))` verifies the
/// document's evidence against that checkout.
pub fn verifier(root: impl Into<String>) -> impl Fn(&Doc) -> Option<Diagnostic> {
    let root = root.into();
    move |doc| evidence::verify(doc, Some(&root), &ProcessGit)
}

/// `path.resolve`: absolute against the working directory, `.` and `..` folded lexically.
fn resolve(input: &str) -> PathBuf {
    let given = Path::new(input);
    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(given)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Node's text for the system errors a root can fail with (`error.message` of `realpathSync`): it
/// walks the path and names the first component it cannot `lstat`.
fn node_message(error: &std::io::Error, path: &Path) -> String {
    let (code, text) = match error.raw_os_error() {
        Some(2) => ("ENOENT", "no such file or directory"),
        Some(13) => ("EACCES", "permission denied"),
        Some(20) => ("ENOTDIR", "not a directory"),
        _ => return error.to_string(),
    };
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if std::fs::symlink_metadata(&prefix).is_err() {
            break;
        }
    }
    format!("{code}: {text}, lstat '{}'", prefix.display())
}

impl Git for ProcessGit {
    fn resolve_root(&self, input: &str) -> Result<String, RootError> {
        let path = resolve(input);
        std::fs::canonicalize(&path)
            .map(|real| real.to_string_lossy().into_owned())
            .map_err(|e| RootError { reason: node_message(&e, &path), requested: path.to_string_lossy().into_owned() })
    }

    fn run(&self, root: &str, args: &[&str]) -> GitRun {
        let output = Command::new("git")
            .args(["--no-replace-objects", "-C", root])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output();
        match output {
            Err(e) => GitRun::Unavailable(format!("spawnSync git {e}")),
            Ok(out) if out.stdout.len() > MAX_OUTPUT_BYTES => GitRun::Unavailable("spawnSync git ENOBUFS".to_owned()),
            Ok(out) => GitRun::Done {
                status: out.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            },
        }
    }

    fn same_entry(&self, left: &str, right: &str) -> Entry {
        let (Ok(a), Ok(b)) = (std::fs::metadata(left), std::fs::metadata(right)) else {
            return Entry::Unknown("entry-missing".to_owned());
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (a.dev(), a.ino()) == (b.dev(), b.ino()) { Entry::Same } else { Entry::Different }
        }
        #[cfg(not(unix))]
        {
            let _ = (a, b);
            match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
                (Ok(x), Ok(y)) if x == y => Entry::Same,
                (Ok(_), Ok(_)) => Entry::Different,
                _ => Entry::Unknown("entry-missing".to_owned()),
            }
        }
    }
}
