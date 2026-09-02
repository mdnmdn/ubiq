//! Pseudo-terminals: opening one, writing to it, resizing it, and getting its output onto the bus.
//!
//! This module is the only place in the application that holds a descriptor or a process. What
//! leaves it is [`Message`] values with a pane ID on them.
//!
//! The reader runs on a thread of its own and sends into an unbounded channel, so a UI that has
//! fallen behind can never stall it — a stalled reader stalls the harness.

use std::io::{Read, Write};
use std::path::Path;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, PtySize, native_pty_system};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::Message;

/// How much output one read may return. Larger chunks mean fewer messages under a flood.
const READ_CHUNK: usize = 8 * 1024;

/// One pane's pseudo-terminal, from the coordinator's side.
pub struct Pty {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/// Open a pseudo-terminal and start `program` in it.
///
/// `TERM` and `COLORTERM` are set here because a harness asks the environment what it may draw,
/// and everything Ubiq shows depends on the answer.
pub fn spawn(
    program: &str,
    args: &[String],
    folder: Option<&Path>,
    cols: u16,
    rows: u16,
) -> Result<(Pty, Box<dyn portable_pty::Child + Send + Sync>)> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening a pseudo-terminal")?;

    let mut command = command_for(program, args);
    if let Some(folder) = folder {
        command.cwd(folder);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");

    let child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("starting {program}"))?;
    tracing::debug!("opened a {cols}x{rows} pseudo-terminal for {program}");
    // The slave is only needed to start the child; holding it would keep the pseudo-terminal open
    // after the harness is gone, and the reader would never see the end of the stream.
    drop(pair.slave);

    let writer = pair
        .master
        .take_writer()
        .context("taking the pseudo-terminal writer")?;
    let killer = child.clone_killer();

    Ok((
        Pty {
            master: pair.master,
            writer,
            killer,
        },
        child,
    ))
}

/// The command a pane runs, built the way the program it names expects to be started.
///
/// **A shell has to be a login shell.** Started as anything else it never sources
/// `.zprofile`/`.zlogin`/`.profile` — where Homebrew's `shellenv` and most `pyenv`, `nvm` and
/// `starship` setup puts things on `PATH` — so tools that are genuinely installed report as
/// `command not found` inside a pane while working in every other terminal on the machine. On Unix
/// a login shell is argv0 prefixed with `-`. `portable-pty` does that prefixing itself, but only
/// for a builder made with `new_default_prog`, which takes no program name and reads the shell out
/// of `SHELL` instead — so the shell being started is handed to it there. Windows has no
/// login/non-login split and nothing about `pwsh.exe` or `cmd.exe` changes.
///
/// Everything else — a harness, or a shell handed a command to run — is built plainly: a `-` on
/// argv0 means "login shell" to a shell and nothing at all to any other program.
fn command_for(program: &str, args: &[String]) -> CommandBuilder {
    #[cfg(unix)]
    if args.is_empty() && crate::shells::is_shell(program) {
        let mut command = CommandBuilder::new_default_prog();
        command.env("SHELL", program);
        return command;
    }

    let mut command = CommandBuilder::new(program);
    for arg in args {
        command.arg(arg);
    }
    command
}

impl Pty {
    /// Forward this pane's output onto the bus until the harness closes the stream.
    pub fn forward_output(&self, pane_id: PaneId, out: Mailbox) -> Result<()> {
        let mut reader = self
            .master
            .try_clone_reader()
            .context("cloning the pseudo-terminal reader")?;

        thread::spawn(move || {
            let mut buffer = vec![0u8; READ_CHUNK];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => {
                        tracing::debug!("pane {pane_id}: the pseudo-terminal stream ended");
                        break;
                    }
                    Ok(n) => {
                        // A window that has gone leaves nothing to draw this pane, so the reader
                        // stops rather than draining the harness into nowhere.
                        let delivered = out.send(Message::TerminalOutput {
                            pane_id,
                            bytes: buffer[..n].to_vec(),
                        });
                        if !delivered {
                            tracing::debug!(
                                "pane {pane_id}: nobody is listening; the reader stops"
                            );
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Set the size the harness believes it has. The kernel signals the process, which redraws.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Kill the harness. Closing a pane is the only thing that does this.
    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

/// Wait for the harness to end and report it, once, on the bus.
pub fn reap(pane_id: PaneId, mut child: Box<dyn portable_pty::Child + Send + Sync>, out: Mailbox) {
    thread::spawn(move || {
        let code = child
            .wait()
            .map(|status| status.exit_code() as i32)
            .unwrap_or(-1);
        out.send(Message::PaneExited { pane_id, code });
    });
}
