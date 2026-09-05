//! One application per config root, and the terminal gets its prompt back.
//!
//! `ubiq some/dir` from a shell starts a *process*, and a second process is not a second
//! application: the catalogue, the host and the windows are process-wide, so two of them disagree
//! about what exists. The first process to reach [`claim`] binds a socket beside the config root
//! and owns the application; every later one finds the socket, hands its paths down it, and exits
//! at once — which is also why the shell prompt comes straight back.
//!
//! The socket lives with the config root rather than in `/tmp`, so `--config-root` is honoured
//! here too: two roots are two applications on purpose, and `just dev` beside an installed bundle
//! is not a collision.
//!
//! Unix only. On Windows every launch owns itself, exactly as it did before.

use std::path::{Path, PathBuf};

/// The socket's name inside the config root.
const NAME: &str = "ubiq.sock";

/// What a launch found: either it owns the application, or another process already does.
pub enum Handoff {
    /// This process is the application. The listener, when there is one, is served by [`serve`].
    Owner(Option<Listener>),
    /// A running application took the paths. This process has nothing left to do.
    Delivered,
}

#[cfg(unix)]
pub struct Listener(std::os::unix::net::UnixListener);

#[cfg(not(unix))]
pub struct Listener(std::convert::Infallible);

/// Hand `paths` to a running application, or become the one that answers.
///
/// A socket left behind by a crash is not a running application: connecting to it fails, and the
/// stale file is removed rather than believed.
#[cfg(unix)]
pub fn claim(root: &Path, paths: &[PathBuf]) -> Handoff {
    use std::io::Write as _;
    use std::os::unix::net::{UnixListener, UnixStream};

    let socket = root.join(NAME);

    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let message: String = paths
            .iter()
            .map(|path| format!("{}\n", path.display()))
            .collect();
        // A write that fails leaves the paths undelivered, but the application on the other end is
        // up: starting a second one beside it would be the worse answer.
        let _ = stream.write_all(message.as_bytes());
        let _ = stream.flush();
        return Handoff::Delivered;
    }

    // Nothing answered: either no socket, or one no process is listening on.
    let _ = std::fs::remove_file(&socket);
    match UnixListener::bind(&socket) {
        Ok(listener) => Handoff::Owner(Some(Listener(listener))),
        // A root that cannot hold a socket — a read-only or exotic filesystem — costs the handoff
        // and nothing else: this process is still the application.
        Err(error) => {
            tracing::warn!("no handoff socket at {}: {error}", socket.display());
            Handoff::Owner(None)
        }
    }
}

#[cfg(not(unix))]
pub fn claim(_root: &Path, _paths: &[PathBuf]) -> Handoff {
    Handoff::Owner(None)
}

/// Answer every later launch, on a thread of its own, for as long as the application lives.
///
/// Each connection is one launch: the paths it wrote, and then end of stream. An empty batch is a
/// bare `ubiq`, which asks for nothing but the window's attention — so it is sent on rather than
/// dropped, and the receiver activates either way.
#[cfg(unix)]
pub fn serve(listener: Listener, paths: flume::Sender<Vec<PathBuf>>) {
    use std::io::Read as _;

    std::thread::spawn(move || {
        for stream in listener.0.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut message = String::new();
            if stream.read_to_string(&mut message).is_err() {
                continue;
            }
            let batch = message
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect();
            if paths.send(batch).is_err() {
                break; // the application is going down
            }
        }
    });
}

#[cfg(not(unix))]
pub fn serve(_listener: Listener, _paths: flume::Sender<Vec<PathBuf>>) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_second_launch_hands_its_paths_to_the_first() {
        let root = tempfile::tempdir().unwrap();

        let Handoff::Owner(Some(listener)) = claim(root.path(), &[]) else {
            panic!("the first launch owns the application");
        };
        let (tx, rx) = flume::unbounded();
        serve(listener, tx);

        let Handoff::Delivered = claim(root.path(), &[PathBuf::from("/work/project")]) else {
            panic!("the second launch hands over");
        };

        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            vec![PathBuf::from("/work/project")]
        );
    }

    #[test]
    fn a_socket_nobody_listens_on_is_stale_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(NAME), "not a socket").unwrap();

        let Handoff::Owner(Some(_listener)) = claim(root.path(), &[]) else {
            panic!("a stale socket is not a running application");
        };
    }
}
