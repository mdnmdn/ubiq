//! The state file a listening drone writes beside its socket.
//!
//! A socket alone answers one question — is something listening — and phase 7 needs a second one:
//! what is it, without connecting to it. So a listening drone writes a small JSON file at the same
//! path with `.sock` swapped for `.json`, and a caller reads that file to discover, list and adopt
//! a running drone instead of guessing whether starting a rival is safe.
//!
//! The file is written after `bind` succeeds and refreshed on a slow cadence so `panes` and
//! `linger` stay roughly true — "roughly" because it is a cache, never the source of truth. The
//! socket is that: [`is_live`] connects to it, and a state file whose socket answers is live
//! whatever its pid says, because a pid can be reused and a socket cannot lie about being open.
//! [`prune`] is what a caller does with the alternative — remove the file and the socket both, so
//! the next caller does not trip over either.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::linger::Linger;

/// What a listening drone says about itself, at rest beside its socket.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DroneState {
    /// `CARGO_PKG_VERSION`, so a mismatch is visible before the handshake catches the schema.
    pub version: String,
    pub pid: u32,
    /// This build's architecture-OS-family triple — see [`crate::triple`].
    pub triple: String,
    /// Unix seconds, for an uptime a caller computes rather than one that goes stale in the file.
    pub started_at: u64,
    pub roots: Vec<String>,
    pub panes: usize,
    /// `Linger`'s `Display`, which is also what `--linger` accepts back — one spelling both ways.
    pub linger: String,
    pub socket: String,
}

/// Where a socket's state file lives: the same path, `.json` for `.sock`.
pub fn state_path(socket: &Path) -> PathBuf {
    socket.with_extension("json")
}

impl DroneState {
    /// Build the record for a drone that is about to listen, or already is.
    pub fn new(socket: &Path, roots: &[PathBuf], panes: usize, linger: Linger) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            triple: crate::triple(),
            started_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs(),
            roots: roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect(),
            panes,
            linger: linger.to_string(),
            socket: socket.to_string_lossy().into_owned(),
        }
    }

    /// Write this record beside `socket`, a temporary file first and a rename to land it: a reader
    /// racing the write sees either the old file or the whole of the new one, never a half one.
    pub fn write(&self, socket: &Path) -> io::Result<()> {
        let path = state_path(socket);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&body)?;
        }
        std::fs::rename(&tmp, &path)
    }

    /// Read one state file back.
    pub fn read(path: &Path) -> io::Result<Self> {
        let body = std::fs::read(path)?;
        serde_json::from_slice(&body)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Remove the state file beside `socket`, if there is one. Never an error: the socket going is
    /// the event that matters, and a state file already gone is not a problem for this to report.
    pub fn remove(socket: &Path) {
        let _ = std::fs::remove_file(state_path(socket));
    }
}

/// Whether a process with this pid is still around.
///
/// Signal `0` sends nothing and asks the kernel only whether it *could* be delivered, which is
/// exactly "does this pid exist and can I see it" — the one syscall this needs, and cheap enough
/// that a libc dependency for it would be a poor trade. It is a hint, not the authority: see
/// [`is_live`] for why the socket is asked first.
#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // A pid this process cannot signal (owned by another user) still answers `0` on most unix
    // kernels, which is the right answer here too: a drone under this same login is the only case
    // this crate ever launches, so "exists" is the only fact being asked for.
    unsafe { kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub fn alive(_pid: u32) -> bool {
    false
}

/// Whether a state file describes a drone that is actually there.
///
/// The socket is the authority and the pid is not asked at all: a pid can be recycled by an
/// unrelated process between one check and the next, on every unix this runs on, while a socket
/// that accepts a connection is a drone answering *right now*, on this same call.
pub fn is_live(state: &DroneState) -> bool {
    UnixStream::connect(&state.socket).is_ok()
}

/// Remove a stale state file and the socket path it names, so the next caller finds neither.
pub fn prune(state: &DroneState) {
    let socket = PathBuf::from(&state.socket);
    tracing::info!("pruning the stale drone at {}", socket.display());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(state_path(&socket));
}

/// Every drone with a state file in `dir`, live ones only and oldest first.
///
/// A stale entry is pruned as it is found rather than merely skipped: a caller that lists is a
/// caller about to decide whether to start one, and a dead file left behind would fail that
/// decision the same way the next caller who lists.
pub fn list(dir: &Path) -> Vec<DroneState> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(state) = DroneState::read(&path) else {
            continue;
        };
        if is_live(&state) {
            found.push(state);
        } else {
            prune(&state);
        }
    }
    found.sort_by_key(|state| state.started_at);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_file_round_trips() {
        let home = tempfile::tempdir().expect("a home for this test");
        let socket = home.path().join("drone.sock");
        let state = DroneState::new(
            &socket,
            &[PathBuf::from("/tmp/project")],
            2,
            Linger::For(Duration::from_secs(600)),
        );
        state.write(&socket).expect("writing the state file");

        let path = state_path(&socket);
        assert!(path.to_string_lossy().ends_with(".json"));
        let read_back = DroneState::read(&path).expect("reading it back");
        assert_eq!(read_back, state);

        DroneState::remove(&socket);
        assert!(!path.exists());
    }

    #[test]
    fn a_state_file_with_no_socket_behind_it_is_not_live() {
        let home = tempfile::tempdir().expect("a home for this test");
        let socket = home.path().join("drone.sock");
        let state = DroneState::new(&socket, &[], 0, Linger::default());
        assert!(
            !is_live(&state),
            "nothing is listening on this socket, so this state is stale"
        );
    }
}
