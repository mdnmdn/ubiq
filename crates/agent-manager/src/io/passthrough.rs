//! The raw-tty pump: forwards bytes verbatim between the local process's
//! stdin/stdout and a PTY master's reader/writer, on background threads.
//!
//! This is the entirety of Phase-1 passthrough I/O (see
//! `_docs/io-modes.md` §passthrough): `am` never parses the byte
//! stream, it just relays it — including Ctrl-C and other control bytes,
//! which reach the child as raw bytes through the tty rather than via a
//! SIGINT handler here.

use std::io::{self, IsTerminal, Read, Write};
use std::thread;

/// RAII guard that restores the local terminal's cooked mode on drop.
///
/// Constructing it (via [`pump`]) enables raw mode only when both stdin and
/// stdout are real terminals; otherwise it is a no-op both to construct and
/// to drop, so headless contexts (tests, pipelines, `cargo test` with no
/// controlling tty) behave like a plain pipe copy.
pub struct RawModeGuard {
    enabled: bool,
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.enabled {
            // Best-effort: nothing useful to do if this fails on the way out.
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

/// Start the bidirectional byte pump between `pty_reader`/`pty_writer` (the
/// PTY master's ends) and the local stdin/stdout, on two detached background
/// threads:
///
/// - `pty_reader → stdout` (write + flush each chunk read).
/// - `stdin → pty_writer` (write + flush each chunk read).
///
/// If the local stdout/stdin are real terminals, raw mode is enabled first
/// so keystrokes (including Ctrl-C) reach the child as bytes rather than
/// being line-buffered or signal-processed locally; the returned
/// [`RawModeGuard`] restores cooked mode when dropped. If not attached to a
/// terminal, raw mode is skipped and this just pumps pipes.
///
/// The pump threads are intentionally not joined: `stdin`'s reader thread in
/// particular may block on a read past the point the child has exited, and
/// that's fine — it's detached and dies with the process.
pub fn pump(pty_reader: Box<dyn Read + Send>, pty_writer: Box<dyn Write + Send>) -> RawModeGuard {
    let is_tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let guard = if is_tty {
        match crossterm::terminal::enable_raw_mode() {
            Ok(()) => RawModeGuard { enabled: true },
            Err(_) => RawModeGuard { enabled: false },
        }
    } else {
        RawModeGuard { enabled: false }
    };

    spawn_pty_to_stdout(pty_reader);
    spawn_stdin_to_pty(pty_writer);

    guard
}

/// `pty master reader → stdout`, on a background thread.
fn spawn_pty_to_stdout(mut pty_reader: Box<dyn Read + Send>) {
    thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buf = [0u8; 8192];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = stdout.flush();
                }
                Err(_) => break,
            }
        }
    });
}

/// `stdin → pty master writer`, on a background thread.
fn spawn_stdin_to_pty(mut pty_writer: Box<dyn Write + Send>) {
    thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 8192];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = pty_writer.flush();
                }
                Err(_) => break,
            }
        }
    });
}
