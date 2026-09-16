//! The unix socket a detached drone is reached through, and the two ends of it.
//!
//! A drone that outlives its link needs somewhere to be found again. That somewhere is a unix
//! socket at mode `0600` under `$XDG_RUNTIME_DIR`, falling back to the cache directory —
//! **filesystem permissions are the whole of the access control**. No port is forwarded and none
//! ever will be: what reaches the socket is what can already open a file owned by this user on
//! this machine, which is a strictly smaller set than anything a key could gate.
//!
//! The path is derived from a hash of the roots the drone was launched with, so two windows
//! opening the same project cannot start rival drones for it — they collide on the socket and the
//! second one attaches to the first.
//!
//! - [`listen`] is the drone: it binds, serves every attach on the same hub, and returns when the
//!   relay's linger expires.
//! - [`attach`] is the client's end: a pure byte relay between this process's standard input and
//!   output and that socket, so reattaching is nothing more than a fresh
//!   `ssh <target> <drone> --attach <sock>`.
//! - [`hold`] is what detaches: a multiplexer session, or `setsid`, holding the listening process
//!   where a user can read its log. **The persistence is the socket, never the multiplexer** — a
//!   `tmux attach` would put a terminal emulator between Ubiq and its binary frames and mangle
//!   them, which is why nothing here ever attaches to one.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use crate::linger::Linger;

/// What an attaching process says before the framing starts.
///
/// One ASCII line, read whole before the stream is handed to the handshake, so no frame is ever
/// split by it. Two verbs live here: `attach`, which carries `--linger` re-asserted on every
/// attach, and `stop`, which asks the drone to end. Both stay off the protocol proper — the
/// carrier family is Ubiq's conversation with the drone, and this is the drone's own two halves
/// talking over a socket only they can open.
const PREAMBLE: &str = "ubiq-drone";

/// The longest preamble that will be read before the stream is given up as not a drone's.
const PREAMBLE_MAX: usize = 128;

/// What the preamble said, once it has been told apart from anything else that might dial a unix
/// socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preamble {
    /// A normal attach. `Some(linger)` when it re-asserted the knob, `None` when it said nothing
    /// and the drone keeps what it has.
    Attach(Option<Linger>),
    /// `--stop`: end this drone, no attach and no handshake to follow.
    Stop,
}

/// Say who is attaching, and what the drone's linger should be from now on.
pub fn write_preamble(writer: &mut impl Write, linger: Option<Linger>) -> io::Result<()> {
    match linger {
        Some(linger) => writeln!(writer, "{PREAMBLE} attach linger={linger}")?,
        None => writeln!(writer, "{PREAMBLE} attach")?,
    }
    writer.flush()
}

/// Say `--stop`, and nothing else: there is no handshake after it, only the one-line answer
/// [`read_stop_answer`] reads back.
pub fn write_stop(writer: &mut impl Write) -> io::Result<()> {
    writeln!(writer, "{PREAMBLE} stop")?;
    writer.flush()
}

/// Read the preamble back, whichever verb it named.
pub fn read_preamble(reader: &mut impl Read) -> io::Result<Preamble> {
    let line = read_line(reader, "the attach named itself")?;
    let Some(rest) = line.strip_prefix(PREAMBLE) else {
        return Err(not_a_drone());
    };
    let rest = rest.trim();
    if let Some(rest) = rest.strip_prefix("attach") {
        return match rest.trim().strip_prefix("linger=") {
            Some(given) => Linger::parse(given)
                .map(|linger| Preamble::Attach(Some(linger)))
                .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why)),
            None => Ok(Preamble::Attach(None)),
        };
    }
    if rest == "stop" {
        return Ok(Preamble::Stop);
    }
    Err(not_a_drone())
}

/// The one line a `--stop` gets back, once the drone has been told to end.
pub fn read_stop_answer(reader: &mut impl Read) -> io::Result<String> {
    read_line(reader, "the stop was answered")
}

fn not_a_drone() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "this is not an attaching drone")
}

/// One line, read byte at a time so nothing beyond it is consumed and left off whatever reads
/// next — the framing that follows an `attach` line depends on that.
fn read_line(reader: &mut impl Read, what_for: &str) -> io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if reader.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("the stream ended before {what_for}"),
            ));
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > PREAMBLE_MAX {
            return Err(not_a_drone());
        }
    }
    Ok(String::from_utf8_lossy(&line).trim().to_string())
}

/// Where the socket for these roots lives.
///
/// `$XDG_RUNTIME_DIR` first — it is per-user, mode `0700` and cleared when the user logs out,
/// which is everything a socket wants. The cache directory is the fallback for a machine that has
/// none (a bare `ssh` into a system without a login session is the ordinary case), and the
/// temporary directory the fallback for a machine with neither.
pub fn socket_path(roots: &[PathBuf]) -> PathBuf {
    state_dir().join(format!("{:016x}.sock", root_hash(roots)))
}

/// Where every drone's socket and state file live: `$XDG_RUNTIME_DIR/ubiq-drone`, with the same
/// fallbacks [`socket_path`] always had. Factored out so [`crate::state::list`] can find every
/// drone on the machine without knowing a single one's roots.
pub fn state_dir() -> PathBuf {
    runtime_dir().join("ubiq-drone")
}

fn runtime_dir() -> PathBuf {
    if let Some(runtime) = non_empty("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime);
    }
    if let Some(cache) = non_empty("XDG_CACHE_HOME") {
        return PathBuf::from(cache);
    }
    if let Some(home) = non_empty("HOME") {
        return PathBuf::from(home).join(".cache");
    }
    std::env::temp_dir()
}

fn non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// FNV-1a over the roots, canonicalised where they exist.
///
/// A hash and not the path itself because a socket path has a length limit an absolute project
/// path would blow through, and because two spellings of one folder must land on one drone.
fn root_hash(roots: &[PathBuf]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for root in roots {
        let resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
        for byte in resolved.to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Whether a program of this name is on `PATH`, for choosing what holds a detached drone.
#[cfg(unix)]
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        std::fs::metadata(&candidate).is_ok_and(|meta| meta.is_file())
    })
}

#[cfg(unix)]
pub use unix::{attach, hold, listen, stop};

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use ubiq_host::carrier::{self, Closer};
    use ubiq_proto::bus;
    use ubiq_proto::carrier::{greet, hello};

    use super::{Preamble, io, on_path, read_preamble, write_preamble};
    use crate::linger::{Linger, Live};
    use crate::relay::{Relay, Root};
    use crate::state::DroneState;

    /// How long [`hold`] waits for the held process to bind before it reports that it did not.
    const APPEARS_WITHIN: Duration = Duration::from_secs(5);

    /// How often the state file beside the socket is rewritten, so `panes` and `linger` stay
    /// roughly true for a `--list` or `--status` that reads it between attaches. Roughly, not
    /// exactly: the socket is the authority on whether the drone is there at all, and this is
    /// only what it says about itself while it is.
    const STATE_REFRESH: Duration = Duration::from_secs(5);

    /// Bind, serve every attach, and return when the drone's linger has expired.
    ///
    /// A path already claimed by a live drone is **adopted, not refused**: [`bind`] returning
    /// `Ok(None)` is success with nothing served, which is what makes
    /// `ubiq-drone --listen … && ubiq-drone --attach …` idempotent rather than a race the second
    /// caller can lose.
    ///
    /// The relay runs on its own thread and the accept loop on another, so the main thread is
    /// free to be the one thing that ends the process: when the relay's countdown runs out it
    /// kills its panes, returns, and the socket and its state file are unlinked here. There is no
    /// other exit, which is the same discipline `carrier` states for the attached case — a second
    /// shutdown path is a second chance to leave a pane running on somebody else's machine.
    pub fn listen(roots: Vec<Root>, path: &Path, linger: Linger) -> io::Result<()> {
        let Some(listener) = bind(path)? else {
            tracing::info!("adopting the drone already listening on {}", path.display());
            return Ok(());
        };
        let live = Arc::new(Live::new(linger));
        tracing::info!("listening on {} with linger {linger}", path.display());

        // The state file and its refresh only ever name the folders, never the ids bound to
        // them: `DroneState` is what a caller lists and adopts by, not what a client attaches to
        // learn a project's id from — that arrives on the wire, once, at `ListProjects`.
        let paths: Vec<PathBuf> = roots.iter().map(|root| root.path.clone()).collect();

        if let Err(error) = DroneState::new(path, &paths, 0, live.linger()).write(path) {
            tracing::warn!(
                "could not write the state file for {}: {error}",
                path.display()
            );
        }

        let (hub, host) = bus::hub();
        let relay = Relay::holding(roots, live.clone());
        let relay = thread::Builder::new()
            .name("ubiq-drone-relay".to_string())
            .spawn(move || relay.run(host))
            .expect("the drone relay thread");

        thread::Builder::new()
            .name("ubiq-drone-accept".to_string())
            .spawn({
                let live = live.clone();
                move || accept(listener, hub, live)
            })
            .expect("the drone accept thread");

        thread::Builder::new()
            .name("ubiq-drone-state".to_string())
            .spawn({
                let path = path.to_path_buf();
                move || refresh_state(&path, &paths, &live)
            })
            .expect("the drone state thread");

        let _ = relay.join();
        let _ = std::fs::remove_file(path);
        DroneState::remove(path);
        Ok(())
    }

    /// Rewrite the state file every [`STATE_REFRESH`], so `panes` and `linger` stay true for
    /// whoever reads it between attaches. It ends on its own, by noticing the socket is gone,
    /// rather than being joined: a state thread with nothing left to refresh has nothing left to
    /// do, and joining it would only make the process's one exit path wait on a sleep.
    fn refresh_state(path: &Path, roots: &[PathBuf], live: &Live) {
        loop {
            thread::sleep(STATE_REFRESH);
            if !path.exists() {
                return;
            }
            let state = DroneState::new(path, roots, live.panes(), live.linger());
            if let Err(error) = state.write(path) {
                tracing::warn!(
                    "could not refresh the state file for {}: {error}",
                    path.display()
                );
            }
        }
    }

    /// The socket, at mode `0600` in a directory at `0700`.
    ///
    /// The directory is what closes the window between binding and the mode being set: a socket
    /// nobody can reach the folder of cannot be connected to whatever its own bits say. `Ok(None)`
    /// is a path a live drone already answers on, for [`listen`] to adopt rather than refuse.
    /// `Ok(Some(_))` is a fresh bind, whether the path was free or the remains of a dead drone
    /// that this removed first.
    fn bind(path: &Path) -> io::Result<Option<UnixListener>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Ok(None);
            }
            tracing::info!(
                "removing the socket a dead drone left at {}",
                path.display()
            );
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Some(listener))
    }

    /// Every attach is a **new host attach**: its own handshake, its own client on the same hub.
    /// Nothing is resumed — the relay re-announces what it is holding, which is what makes a
    /// reattach need no state the two ends have to agree about.
    fn accept(listener: UnixListener, hub: bus::Hub, live: Arc<Live>) {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    tracing::warn!("an attach failed to arrive: {error}");
                    continue;
                }
            };
            let hub = hub.clone();
            let live = live.clone();
            thread::Builder::new()
                .name("ubiq-drone-session".to_string())
                .spawn(move || serve_one(stream, hub, live))
                .expect("the drone session thread");
        }
    }

    fn serve_one(mut stream: UnixStream, hub: bus::Hub, live: Arc<Live>) {
        let asked = match read_preamble(&mut stream) {
            Ok(Preamble::Attach(asked)) => asked,
            Ok(Preamble::Stop) => {
                tracing::info!("--stop asked over the socket: ending this drone");
                let _ = writeln!(stream, "stopped");
                let _ = stream.flush();
                live.stop();
                return;
            }
            Err(error) => {
                tracing::warn!("refusing a connection that is not an attach: {error}");
                return;
            }
        };
        // Re-asserted here, before anything else: changing the knob costs no restart, and a
        // drone that is about to hold panes should already know for how long.
        if let Some(asked) = asked {
            tracing::info!("the attaching client asked for linger {asked}");
            live.set_linger(asked);
        }

        let (Ok(mut reader), Ok(writer), Ok(closing)) =
            (stream.try_clone(), stream.try_clone(), stream.try_clone())
        else {
            tracing::warn!("could not split the attached stream");
            return;
        };
        let introduction = hello(env!("CARGO_PKG_VERSION"), &crate::capabilities());
        if let Err(refusal) = greet(&mut reader, &mut stream, &introduction) {
            tracing::warn!("the attaching client refused this drone: {refusal}");
            return;
        }
        carrier::pump(
            Box::new(reader),
            Box::new(writer),
            Box::new(Hangup(closing)),
            hub,
        );
        tracing::info!("an attached client has gone");
    }

    /// What ends the read direction when the writer gives up — see `carrier::Closer`. A socket
    /// sends no end-of-stream while a reader still holds a clone of it, so the shutdown is the
    /// only thing that unblocks one.
    struct Hangup(UnixStream);

    impl Closer for Hangup {
        fn close(&self) {
            let _ = self.0.shutdown(std::net::Shutdown::Both);
        }
    }

    /// The other end: this process's standard input and output, relayed to a listening drone.
    ///
    /// A **pure byte relay** and nothing else — it neither frames nor parses, so a schema it has
    /// never heard of passes through it unchanged and an old attach never has to be replaced to
    /// carry a new conversation.
    pub fn attach(path: &Path, linger: Option<Linger>) -> io::Result<()> {
        let mut stream = UnixStream::connect(path)?;
        write_preamble(&mut stream, linger)?;

        let mut up = stream.try_clone()?;
        let upward = thread::Builder::new()
            .name("ubiq-drone-attach-in".to_string())
            .spawn(move || {
                let _ = std::io::copy(&mut std::io::stdin().lock(), &mut up);
                // The drone is owed the end of the stream, or it holds this link open against a
                // client that has already gone.
                let _ = up.shutdown(std::net::Shutdown::Write);
            })
            .expect("the attach's input thread");

        // On this thread, so the process ends when the drone does. Standard output is flushed per
        // write for the reason `carrier::WriteHalf` gives: a binary frame carries no newline, and
        // a line-buffered answer would sit in the buffer and look like a hang.
        let mut buffer = [0u8; 32 * 1024];
        let stdout = std::io::stdout();
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut out = stdout.lock();
                    if out.write_all(&buffer[..n]).is_err() || out.flush().is_err() {
                        break;
                    }
                }
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = upward.join();
        Ok(())
    }

    /// Ask a listening drone to end, and return what it said back.
    ///
    /// One connection, one line out, one line in: there is no handshake and no attach after a
    /// `stop`, because there is nothing left here to carry on with — the relay's own tick is what
    /// actually ends the process, and this only asks for that tick to come.
    pub fn stop(path: &Path) -> io::Result<String> {
        let mut stream = UnixStream::connect(path)?;
        super::write_stop(&mut stream)?;
        super::read_stop_answer(&mut stream)
    }

    /// Detach: hand the listening process to something that will hold it, and return once its
    /// socket is there to be attached to.
    ///
    /// A multiplexer is used **only** to hold the process where a user can read its log — never
    /// to talk to it. `setsid` is the answer where there is none, and a bare spawn the answer
    /// where there is no `setsid` either, which leaves the drone bound to the login that started
    /// it and is said out loud rather than hidden.
    pub fn hold(exe: &Path, argv: &[String], path: &Path) -> io::Result<()> {
        let command = shell_line(exe, argv);
        let session = format!(
            "ubiq-drone-{}",
            path.file_stem().unwrap_or_default().to_string_lossy()
        );

        // A multiplexer returns as soon as its session exists, so it is worth reaping. `setsid`
        // and a bare spawn *become* the drone: waiting on either would be waiting on the whole
        // session, which is the opposite of detaching.
        let (mut held, returns) = if on_path("tmux") {
            let mut tmux = Command::new("tmux");
            tmux.args(["new", "-d", "-s", &session, &command]);
            (tmux, true)
        } else if on_path("screen") {
            let mut screen = Command::new("screen");
            screen.args(["-dmS", &session, "sh", "-c", &command]);
            (screen, true)
        } else if on_path("setsid") {
            let mut setsid = Command::new("setsid");
            setsid.arg(exe).args(argv);
            (setsid, false)
        } else {
            tracing::warn!("no tmux, screen or setsid: this drone dies with the login");
            let mut bare = Command::new(exe);
            bare.args(argv);
            (bare, false)
        };
        let mut child = held
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if returns {
            let _ = child.wait();
        }

        let deadline = Instant::now() + APPEARS_WITHIN;
        while Instant::now() < deadline {
            if UnixStream::connect(path).is_ok() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("the held drone never bound {}", path.display()),
        ))
    }

    /// The command line a multiplexer is given, quoted for the shell it runs it under.
    fn shell_line(exe: &Path, argv: &[String]) -> String {
        let mut line = quote(&exe.to_string_lossy());
        for arg in argv {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        line
    }

    fn quote(word: &str) -> String {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

#[cfg(not(unix))]
mod elsewhere {
    //! The detached lifecycle is unix-only. The socket, its mode bits and the multiplexer that
    //! holds the process are the whole of the mechanism, and none of the three has a Windows
    //! spelling that would mean the same thing — so the crate builds there and says so rather
    //! than pretending.

    use std::path::Path;

    use super::*;
    use crate::linger::Linger;
    use crate::relay::Root;

    fn unsupported(what: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{what} needs a unix socket, and this is not a unix"),
        )
    }

    pub fn listen(_roots: Vec<Root>, _path: &Path, _linger: Linger) -> io::Result<()> {
        Err(unsupported("--listen"))
    }

    pub fn attach(_path: &Path, _linger: Option<Linger>) -> io::Result<()> {
        Err(unsupported("--attach"))
    }

    pub fn hold(_exe: &Path, _argv: &[String], _path: &Path) -> io::Result<()> {
        Err(unsupported("detaching"))
    }

    pub fn stop(_path: &Path) -> io::Result<String> {
        Err(unsupported("--stop"))
    }
}

#[cfg(not(unix))]
pub use elsewhere::{attach, hold, listen, stop};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_root_is_one_socket() {
        let here = PathBuf::from(".");
        let root = std::env::current_dir().expect("a working directory");
        // Two spellings of one folder are one drone: the path is canonicalised before it is
        // hashed, which is what stops two windows starting rivals for one project.
        assert_eq!(
            socket_path(&[here]),
            socket_path(std::slice::from_ref(&root))
        );
        assert_ne!(socket_path(&[root]), socket_path(&[PathBuf::from("/")]));
        assert!(
            socket_path(&[PathBuf::from("/")])
                .to_string_lossy()
                .ends_with(".sock")
        );
    }

    #[test]
    fn the_preamble_carries_the_linger_and_refuses_anything_else() {
        let mut said = Vec::new();
        write_preamble(
            &mut said,
            Some(Linger::For(std::time::Duration::from_secs(30))),
        )
        .expect("writing the preamble");
        assert_eq!(
            read_preamble(&mut said.as_slice()).expect("reading it back"),
            Preamble::Attach(Some(Linger::For(std::time::Duration::from_secs(30))))
        );

        let mut quiet = Vec::new();
        write_preamble(&mut quiet, None).expect("writing the preamble");
        assert_eq!(
            read_preamble(&mut quiet.as_slice()).expect("reading"),
            Preamble::Attach(None)
        );

        assert!(read_preamble(&mut b"GET / HTTP/1.1\n".as_slice()).is_err());
    }

    #[test]
    fn the_preamble_s_second_verb_asks_to_stop() {
        let mut said = Vec::new();
        write_stop(&mut said).expect("writing the stop preamble");
        assert_eq!(
            read_preamble(&mut said.as_slice()).expect("reading it back"),
            Preamble::Stop
        );
    }
}
