//! The coordinator: it starts harnesses, supervises them, and answers the bus.
//!
//! It runs on a thread of its own and shares nothing with the window but the channel pair in
//! [`crate::bus`]. It renders nothing and has no opinion about layout or colour — everything here
//! is a pane ID, a pseudo-terminal and a process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;

use uuid::Uuid;

use crate::bus::CoordinatorEnd;
use crate::messages::{Message, WorkspaceInfo};
use crate::pty::{self, Pty};

/// The geometry a pane starts at, before the emulator has measured its own bounds and said what it
/// really is. The harness is told the truth a frame later, by [`Message::TerminalResize`].
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// Start the coordinator on its own thread. It ends when the bus closes, which is when the last
/// window has gone.
pub fn start(end: CoordinatorEnd) {
    thread::Builder::new()
        .name("ubiq-coordinator".to_string())
        .spawn(move || Coordinator::new(end.to_ui).run(end.from_ui))
        .expect("the coordinator thread");
}

struct Coordinator {
    to_ui: flume::Sender<Message>,
    panes: HashMap<Uuid, Pty>,
    /// Which pane the UI says has focus. Exactly one, or none before the first pane exists.
    focused: Option<Uuid>,
}

impl Coordinator {
    fn new(to_ui: flume::Sender<Message>) -> Self {
        Self {
            to_ui,
            panes: HashMap::new(),
            focused: None,
        }
    }

    fn run(mut self, from_ui: flume::Receiver<Message>) {
        while let Ok(message) = from_ui.recv() {
            self.dispatch(message);
        }
    }

    fn dispatch(&mut self, message: Message) {
        match message {
            Message::SpawnWorkspace {
                session_id,
                agent_type,
                args,
                folder,
            } => self.spawn_workspace(session_id, agent_type, args, folder),

            Message::TerminalInput { pane_id, bytes } => {
                if let Some(pane) = self.panes.get_mut(&pane_id)
                    && let Err(error) = pane.write(&bytes)
                {
                    let _ = self.to_ui.send(Message::PaneError {
                        pane_id,
                        error: error.to_string(),
                    });
                }
            }

            // A resize for a pane that has gone is ignored: the geometry has nowhere to land.
            Message::TerminalResize {
                pane_id,
                cols,
                rows,
            } => {
                if let Some(pane) = self.panes.get(&pane_id)
                    && let Err(error) = pane.resize(cols, rows)
                {
                    let _ = self.to_ui.send(Message::PaneError {
                        pane_id,
                        error: error.to_string(),
                    });
                }
            }

            Message::Focus { pane_id } => self.focused = Some(pane_id),

            Message::CloseWorkspace { pane_id } => {
                if let Some(mut pane) = self.panes.remove(&pane_id) {
                    tracing::info!("closing pane {pane_id}, killing its harness");
                    pane.kill();
                }
                if self.focused == Some(pane_id) {
                    self.focused = None;
                }
            }

            // Response-direction variants are never received here. Dropping one silently would
            // hide a wiring mistake, so it is named.
            other => {
                tracing::warn!("the coordinator was sent a message only it may send: {other:?}")
            }
        }
    }

    fn spawn_workspace(
        &mut self,
        session_id: Uuid,
        agent_type: Option<String>,
        args: Vec<String>,
        folder: Option<String>,
    ) {
        let pane_id = Uuid::new_v4();
        let agent_type = agent_type.unwrap_or_else(default_agent_type);
        let path = folder.as_ref().map(PathBuf::from);

        let spawned = pty::spawn(
            &agent_type,
            &args,
            path.as_deref(),
            INITIAL_COLS,
            INITIAL_ROWS,
        );
        let (pane, child) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                tracing::error!("pane {pane_id}: starting {agent_type} failed: {error:#}");
                let _ = self.to_ui.send(Message::PaneError {
                    pane_id,
                    error: error.to_string(),
                });
                return;
            }
        };
        tracing::info!("pane {pane_id}: started {agent_type} in session {session_id}");

        if let Err(error) = pane.forward_output(pane_id, self.to_ui.clone()) {
            let _ = self.to_ui.send(Message::PaneError {
                pane_id,
                error: error.to_string(),
            });
            return;
        }
        pty::reap(pane_id, child, self.to_ui.clone());
        self.panes.insert(pane_id, pane);

        let _ = self.to_ui.send(Message::WorkspaceSpawned {
            workspace: WorkspaceInfo {
                id: pane_id,
                session_id,
                agent_type,
                folder,
                cols: INITIAL_COLS,
                rows: INITIAL_ROWS,
                running: true,
            },
        });
    }
}

/// What a session starts when it is not told what to start: the user's own shell.
fn default_agent_type() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}
