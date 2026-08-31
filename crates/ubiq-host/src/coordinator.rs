//! The coordinator: it starts harnesses, supervises them, and answers the bus.
//!
//! It runs on a thread of its own and shares nothing with the window but the channel pair in
//! [`ubiq_proto::bus`]. It renders nothing and has no opinion about layout or colour — everything here
//! is a pane ID, a pseudo-terminal and a process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;

use ubiq_proto::bus::{ClientId, FromClient, HostEnd, To};
use ubiq_proto::ids::{PaneId, SessionId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use crate::pty::{self, Pty};

/// The geometry a pane starts at, before the emulator has measured its own bounds and said what it
/// really is. The harness is told the truth a frame later, by [`Message::TerminalResize`].
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// Start the coordinator on its own thread. One per process, started before the first window: the
/// catalogue it will own is process-wide, and two of them would disagree about what exists.
///
/// It ends when the hub and every client have gone.
pub fn start(host: HostEnd) {
    thread::Builder::new()
        .name("ubiq-coordinator".to_string())
        .spawn(move || Coordinator::new(host).run())
        .expect("the coordinator thread");
}

struct Coordinator {
    host: HostEnd,
    panes: HashMap<PaneId, Pty>,
    /// Which window owns which pane, recorded when the pane is spawned. This is the whole routing
    /// table: everything a pane emits goes to its owner, and nobody else may drive it.
    owners: HashMap<PaneId, ClientId>,
    /// Which pane each window says has focus. Exactly one per window — two attached windows have
    /// two focused panes, and neither is more focused than the other.
    focused: HashMap<ClientId, PaneId>,
}

impl Coordinator {
    fn new(host: HostEnd) -> Self {
        Self {
            host,
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
        }
    }

    fn run(mut self) {
        while let Ok(event) = self.host.recv() {
            match event {
                FromClient::Connected(client) => tracing::debug!("{client} attached"),
                FromClient::Said { client, message } => self.dispatch(client, message),
                FromClient::Gone(client) => self.client_gone(client),
            }
        }
    }

    /// A window has gone. Everything it owned goes with it.
    ///
    /// This used to happen by itself: a closed window dropped its whole bus, the coordinator thread
    /// ended, and the pseudo-terminals went with it. One host outlives every window, so nothing
    /// drops now and the reaping has to be deliberate — without this, every closed window leaves a
    /// live harness behind.
    fn client_gone(&mut self, client: ClientId) {
        self.focused.remove(&client);
        let owned: Vec<PaneId> = self
            .owners
            .iter()
            .filter(|(_, owner)| **owner == client)
            .map(|(pane_id, _)| *pane_id)
            .collect();

        // Taking the pseudo-terminal out of the map is most of the work: dropping it closes the
        // master, the kernel hangs up the slave, and the harness goes. The kill is the guarantee
        // for anything that ignores the hang-up.
        for pane_id in owned {
            self.owners.remove(&pane_id);
            if let Some(mut pane) = self.panes.remove(&pane_id) {
                tracing::info!("{client} has gone; killing the harness in pane {pane_id}");
                pane.kill();
            }
        }
    }

    /// Whether this window is the one that owns the pane. A message about somebody else's pane is
    /// a wiring mistake, not something to act on.
    fn owns(&self, client: ClientId, pane_id: PaneId) -> bool {
        match self.owners.get(&pane_id) {
            Some(owner) if *owner == client => true,
            Some(_) => {
                tracing::warn!("{client} sent a message about pane {pane_id}, which it does not own");
                false
            }
            // The pane has already gone; its last messages are in flight behind it.
            None => false,
        }
    }

    fn dispatch(&mut self, client: ClientId, message: Message) {
        match message {
            Message::SpawnWorkspace {
                session_id,
                agent_type,
                args,
                folder,
            } => self.spawn_workspace(client, session_id, agent_type, args, folder),

            Message::TerminalInput { pane_id, bytes } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                if let Some(pane) = self.panes.get_mut(&pane_id)
                    && let Err(error) = pane.write(&bytes)
                {
                    self.host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            // A resize for a pane that has gone is ignored: the geometry has nowhere to land.
            Message::TerminalResize {
                pane_id,
                cols,
                rows,
            } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                if let Some(pane) = self.panes.get(&pane_id)
                    && let Err(error) = pane.resize(cols, rows)
                {
                    self.host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            Message::Focus { pane_id } => {
                if self.owns(client, pane_id) {
                    self.focused.insert(client, pane_id);
                }
            }

            Message::CloseWorkspace { pane_id } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                self.owners.remove(&pane_id);
                if let Some(mut pane) = self.panes.remove(&pane_id) {
                    tracing::info!("closing pane {pane_id}, killing its harness");
                    pane.kill();
                }
                if self.focused.get(&client) == Some(&pane_id) {
                    self.focused.remove(&client);
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
        client: ClientId,
        session_id: SessionId,
        agent_type: Option<String>,
        args: Vec<String>,
        folder: Option<String>,
    ) {
        let pane_id = PaneId::generate();
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
                self.host.send(
                    To::Client(client),
                    Message::PaneError {
                        pane_id,
                        error: error.to_string(),
                    },
                );
                return;
            }
        };
        tracing::info!("pane {pane_id}: started {agent_type} in session {session_id} for {client}");

        // The owner is recorded before the pane is announced, so nothing can arrive about a pane
        // the routing table has never heard of.
        self.owners.insert(pane_id, client);
        let mailbox = self.host.mailbox(To::Client(client));

        if let Err(error) = pane.forward_output(pane_id, mailbox.clone()) {
            self.owners.remove(&pane_id);
            mailbox.send(Message::PaneError {
                pane_id,
                error: error.to_string(),
            });
            return;
        }
        pty::reap(pane_id, child, mailbox.clone());
        self.panes.insert(pane_id, pane);

        mailbox.send(Message::WorkspaceSpawned {
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
