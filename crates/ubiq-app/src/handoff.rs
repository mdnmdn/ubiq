//! One application per config root, and the terminal gets its prompt back.
//!
//! `ubiq some/dir` from a shell starts a *process*, and a second process is not a second
//! application: the catalogue, the host and the windows are process-wide, so two of them disagree
//! about what exists. The first process to reach [`claim`] opens a door beside the config root and
//! owns the application; every later one finds the door, hands its paths through it, and exits at
//! once — which is also why the shell prompt comes straight back.
//!
//! The door lives with the config root rather than in `/tmp`, so `--config-root` is honoured here
//! too: two roots are two applications on purpose. Its name carries the executable's path as well,
//! so two *builds* are two applications too: `cargo run` beside an installed bundle, both on the
//! default root, each owns itself. Only a relaunch of the same binary — which is what `ubiq
//! some/dir` from the shell, or Explorer's *Open in Ubiq*, is — hands over.
//!
//! **On Unix the door is a socket.** **On Windows it is a loopback port and a file naming it.** The
//! owner binds `127.0.0.1:0` and writes the port and a random token into a file of the same name a
//! socket would have; a later launch reads the file, dials the port, sends the token ahead of its
//! paths, and counts the paths delivered only once the owner acknowledges them. The token is what
//! keeps any other local process from posting paths into the window, and the acknowledgement is
//! what keeps a port the system has since handed to somebody else from swallowing a launch: a
//! file left behind by a crash names a port nobody answers on, or one that answers wrongly, and
//! either way it is stale and replaced rather than believed. The framing after the token line is
//! the socket's, one path per line.

use std::path::{Path, PathBuf};

/// The door's name inside the config root, one per executable.
///
/// The path is hashed rather than spelled out because it is a path, and because the only question
/// asked of it is whether two launches are the same build.
#[cfg(any(unix, windows))]
fn name() -> String {
    use std::hash::{Hash as _, Hasher as _};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::env::current_exe().ok().hash(&mut hasher);
    let kind = if cfg!(windows) { "port" } else { "sock" };
    format!("ubiq-{:016x}.{kind}", hasher.finish())
}

/// What a launch found: either it owns the application, or another process already does.
pub enum Handoff {
    /// This process is the application. The listener, when there is one, is served by [`serve`].
    Owner(Option<Listener>),
    /// A running application took the paths. This process has nothing left to do.
    Delivered,
}

#[cfg(unix)]
pub struct Listener(std::os::unix::net::UnixListener);

/// The bound port, and the token a launch has to present before anything it sends is believed.
#[cfg(windows)]
pub struct Listener {
    socket: std::net::TcpListener,
    token: String,
}

#[cfg(not(any(unix, windows)))]
pub struct Listener(std::convert::Infallible);

/// One launch's paths, one per line, blank lines dropped. The framing both doors share.
#[cfg(any(unix, windows))]
fn batch<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<PathBuf> {
    lines
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Hand `paths` to a running application, or become the one that answers.
///
/// A socket left behind by a crash is not a running application: connecting to it fails, and the
/// stale file is removed rather than believed.
#[cfg(unix)]
pub fn claim(root: &Path, paths: &[PathBuf]) -> Handoff {
    use std::io::Write as _;
    use std::os::unix::net::{UnixListener, UnixStream};

    let socket = root.join(name());

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

/// Hand `paths` to a running application, or become the one that answers.
///
/// Only an acknowledged delivery counts. Anything short of one — no file, a file naming a port
/// nobody listens on, a port that answers but not as this application — means no application is
/// up, and this launch binds a port of its own and overwrites the file.
#[cfg(windows)]
pub fn claim(root: &Path, paths: &[PathBuf]) -> Handoff {
    use std::net::{Ipv4Addr, TcpListener};

    let file = root.join(name());

    if let Some((port, token)) = read_record(&file)
        && deliver(port, &token, paths)
    {
        return Handoff::Delivered;
    }

    // A root that cannot hold the file, or a machine that will not give out a loopback port, costs
    // the handoff and nothing else: this process is still the application.
    let socket = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!("no handoff port: {error}");
            return Handoff::Owner(None);
        }
    };
    let port = match socket.local_addr() {
        Ok(address) => address.port(),
        Err(error) => {
            tracing::warn!("no handoff port: {error}");
            return Handoff::Owner(None);
        }
    };
    let token = token();
    if let Err(error) = write_record(root, &file, port, &token) {
        tracing::warn!("no handoff file at {}: {error}", file.display());
        return Handoff::Owner(None);
    }
    Handoff::Owner(Some(Listener { socket, token }))
}

#[cfg(not(any(unix, windows)))]
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
            if paths.send(batch(message.lines())).is_err() {
                break; // the application is going down
            }
        }
    });
}

/// Answer every later launch, on a thread of its own, for as long as the application lives.
///
/// The Unix contract, behind a token line: a connection whose first line is not this run's token is
/// dropped unanswered, and one that presents it is acknowledged before its batch goes on — the
/// acknowledgement is how the launch on the other end knows it reached this application and not a
/// stranger on a reused port.
#[cfg(windows)]
pub fn serve(listener: Listener, paths: flume::Sender<Vec<PathBuf>>) {
    use std::io::{Read as _, Write as _};

    std::thread::spawn(move || {
        for stream in listener.socket.incoming() {
            let Ok(mut stream) = stream else { continue };
            // A launch that connects and then says nothing must not hold every later one up.
            let _ = stream.set_read_timeout(Some(ACK_PATIENCE));
            let _ = stream.set_write_timeout(Some(ACK_PATIENCE));
            let mut message = String::new();
            if (&stream).take(LIMIT).read_to_string(&mut message).is_err() {
                continue;
            }
            let Some(arrived) = admit(&message, &listener.token) else {
                tracing::warn!(
                    "refused a handoff connection that did not present this run's token"
                );
                continue;
            };
            let _ = stream.write_all(ACK.as_bytes());
            let _ = stream.flush();
            if paths.send(arrived).is_err() {
                break; // the application is going down
            }
        }
    });
}

#[cfg(not(any(unix, windows)))]
pub fn serve(_listener: Listener, _paths: flume::Sender<Vec<PathBuf>>) {}

/// What the owner answers a launch it believed. Anything else is not this application.
#[cfg(windows)]
const ACK: &str = "ubiq\n";

/// How long a launch waits for a port to accept. A live owner's kernel accepts at once even while
/// the application is busy, so this only ever runs out on a stale port — where Windows would
/// otherwise retry a refused loopback connection for the better part of two seconds on every cold
/// start.
#[cfg(windows)]
const CONNECT_PATIENCE: std::time::Duration = std::time::Duration::from_millis(300);

/// How long either end waits for the other once connected.
#[cfg(windows)]
const ACK_PATIENCE: std::time::Duration = std::time::Duration::from_secs(3);

/// The most one launch may send. Paths, not payloads.
#[cfg(windows)]
const LIMIT: u64 = 1 << 20;

/// The port and token a running owner wrote, if the file is there and reads as one.
#[cfg(windows)]
fn read_record(file: &Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse().ok()?;
    let token = lines.next()?.trim().to_string();
    (!token.is_empty()).then_some((port, token))
}

/// Written beside and renamed over, so a launch reading at the same moment sees the old record or
/// the new one and never half of either.
#[cfg(windows)]
fn write_record(root: &Path, file: &Path, port: u16, token: &str) -> std::io::Result<()> {
    let scratch = root.join(format!("{}.tmp", name()));
    std::fs::write(&scratch, format!("{port}\n{token}\n"))?;
    std::fs::rename(&scratch, file)
}

/// A token nobody else on the machine can guess: two keyed SipHash outputs, each under a key
/// `RandomState` draws from the operating system's randomness. No dependency, and nothing here
/// needs more than that the token is unpredictable to another process.
#[cfg(windows)]
fn token() -> String {
    use std::hash::BuildHasher as _;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    (0..2u8)
        .map(|round| {
            let hash = std::collections::hash_map::RandomState::new().hash_one((
                nanos,
                std::process::id(),
                round,
            ));
            format!("{hash:016x}")
        })
        .collect()
}

/// Dial the owner, present the token and the paths, and wait for it to say it took them.
#[cfg(windows)]
fn deliver(port: u16, token: &str, paths: &[PathBuf]) -> bool {
    use std::io::{Read as _, Write as _};
    use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream};

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, CONNECT_PATIENCE) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(ACK_PATIENCE));
    let _ = stream.set_write_timeout(Some(ACK_PATIENCE));
    allow_foreground();

    let mut message = format!("{token}\n");
    for path in paths {
        message.push_str(&format!("{}\n", path.display()));
    }
    if stream.write_all(message.as_bytes()).is_err() || stream.shutdown(Shutdown::Write).is_err() {
        return false;
    }

    let mut answer = String::new();
    let _ = (&stream).take(ACK.len() as u64).read_to_string(&mut answer);
    answer == ACK
}

/// Pass this launch's right to take the foreground on to whichever process asks next.
///
/// Windows lets a process bring its window forward only when the user just interacted with it,
/// and a launch from Explorer or a terminal is that process — not the owner it hands over to. So
/// without this the owner's `activate` flashes a taskbar button instead of raising the window.
/// Linked straight from `user32`, which every windowed process already carries; a failure is a
/// window that stays behind, which is exactly the state without the call.
#[cfg(windows)]
fn allow_foreground() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }
    const ASFW_ANY: u32 = u32::MAX;
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// The batch a connection carries, or `None` when its first line is not `token`.
#[cfg(windows)]
fn admit(message: &str, token: &str) -> Option<Vec<PathBuf>> {
    let mut lines = message.lines();
    if lines.next()?.trim() != token {
        return None;
    }
    Some(batch(lines))
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;

    fn project() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\work\project")
        } else {
            PathBuf::from("/work/project")
        }
    }

    #[test]
    fn the_second_launch_hands_its_paths_to_the_first() {
        let root = tempfile::tempdir().unwrap();

        let Handoff::Owner(Some(listener)) = claim(root.path(), &[]) else {
            panic!("the first launch owns the application");
        };
        let (tx, rx) = flume::unbounded();
        serve(listener, tx);

        let Handoff::Delivered = claim(root.path(), &[project()]) else {
            panic!("the second launch hands over");
        };

        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            vec![project()]
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_socket_nobody_listens_on_is_stale_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(name()), "not a socket").unwrap();

        let Handoff::Owner(Some(_listener)) = claim(root.path(), &[]) else {
            panic!("a stale socket is not a running application");
        };
    }

    /// A port that was free a moment ago: bound, read, and let go.
    #[cfg(windows)]
    fn free_port() -> u16 {
        let socket = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket.local_addr().unwrap().port()
    }

    #[test]
    #[cfg(windows)]
    fn a_port_nobody_listens_on_is_stale_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(name());
        std::fs::write(&file, format!("{}\nstale\n", free_port())).unwrap();

        let Handoff::Owner(Some(listener)) = claim(root.path(), &[]) else {
            panic!("a stale port is not a running application");
        };
        let (_, token) = read_record(&file).expect("the file is rewritten");
        assert_eq!(token, listener.token);
        assert_ne!(token, "stale");
    }

    #[test]
    #[cfg(windows)]
    fn a_file_that_is_not_a_record_is_stale_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(name()), "not a record").unwrap();

        let Handoff::Owner(Some(_listener)) = claim(root.path(), &[]) else {
            panic!("an unreadable record is not a running application");
        };
    }

    /// The port went to some other program after a crash: it accepts, and says nothing back.
    #[test]
    #[cfg(windows)]
    fn a_port_that_answers_without_the_acknowledgement_is_not_the_application() {
        use std::io::Read as _;

        let root = tempfile::tempdir().unwrap();
        let stranger = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = stranger.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = stranger.accept() {
                let mut sink = String::new();
                let _ = stream.read_to_string(&mut sink);
            }
        });
        std::fs::write(root.path().join(name()), format!("{port}\nsome-token\n")).unwrap();

        let Handoff::Owner(Some(_listener)) = claim(root.path(), &[project()]) else {
            panic!("a stranger on the port is not a running application");
        };
    }

    #[test]
    #[cfg(windows)]
    fn a_connection_without_the_token_is_refused() {
        let root = tempfile::tempdir().unwrap();

        let Handoff::Owner(Some(listener)) = claim(root.path(), &[]) else {
            panic!("the first launch owns the application");
        };
        let (tx, rx) = flume::unbounded();
        serve(listener, tx);

        let (port, _) = read_record(&root.path().join(name())).unwrap();
        assert!(!deliver(port, "not-the-token", &[project()]));
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "nothing reaches the window without the token"
        );
    }

    #[test]
    #[cfg(windows)]
    fn admit_reads_the_token_line_and_then_the_unix_framing() {
        assert_eq!(
            admit("secret\nC:\\a\n\nC:\\b\n", "secret"),
            Some(vec![PathBuf::from(r"C:\a"), PathBuf::from(r"C:\b")])
        );
        assert_eq!(admit("secret\n", "secret"), Some(Vec::new()));
        assert_eq!(admit("wrong\nC:\\a\n", "secret"), None);
        assert_eq!(admit("", "secret"), None);
    }

    #[test]
    #[cfg(windows)]
    fn two_tokens_differ() {
        assert_ne!(token(), token());
        assert_eq!(token().len(), 32);
    }
}
