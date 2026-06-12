use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize, PtySystem};
use uuid::Uuid;

use crate::agent::AgentRegistry;
use crate::bus::SharedBus;
use crate::messages::*;

pub struct Orchestrator {
    bus: SharedBus,
    agent_registry: AgentRegistry,
    sessions: HashMap<SessionId, Session>,
    workspaces: HashMap<WorkspaceId, WorkspaceInfo>,
    workspace_io: HashMap<WorkspaceId, WorkspaceIO>,
}

struct Session {
    info: SessionInfo,
    workspace_ids: HashSet<WorkspaceId>,
}

struct WorkspaceIO {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
}

impl Orchestrator {
    pub fn new(bus: SharedBus, agent_registry: AgentRegistry) -> Self {
        Self {
            bus,
            agent_registry,
            sessions: HashMap::new(),
            workspaces: HashMap::new(),
            workspace_io: HashMap::new(),
        }
    }

    pub fn handle_message(&mut self, msg: BusMessage) {
        match msg {
            BusMessage::ListSessions => self.list_sessions(),
            BusMessage::CreateSession {
                name,
                agent_type,
                home_folder,
            } => self.create_session(name, agent_type, home_folder),
            BusMessage::ListAgentTypes => self.list_agent_types(),
            BusMessage::AttachToSession { session_id } => self.attach_session(session_id),
            BusMessage::DetachFromSession { session_id } => self.detach_session(session_id),
            BusMessage::SpawnWorkspace {
                session_id,
                agent_type,
                folder,
            } => self.spawn_workspace(session_id, agent_type, folder),
            BusMessage::TerminalInput {
                workspace_id,
                bytes,
            } => self.handle_terminal_input(workspace_id, bytes),
            BusMessage::TerminalResize {
                workspace_id,
                cols,
                rows,
            } => self.handle_terminal_resize(workspace_id, cols, rows),
            _ => {}
        }
    }

    // ── Session management ────────────────────────────────────────

    fn list_sessions(&self) {
        let sessions: Vec<SessionInfo> = self.sessions.values().map(|s| s.info.clone()).collect();
        let _ = self.bus.send(&BusMessage::SessionList { sessions });
    }

    fn create_session(
        &mut self,
        name: String,
        agent_type: String,
        home_folder: Option<String>,
    ) {
        if !self.agent_registry.has(&agent_type) {
            let _ = self
                .bus
                .send_error(&format!("Unknown agent type: {}", agent_type));
            return;
        }

        let home = home_folder.unwrap_or_else(|| "./_workspace".to_string());
        let home_path = PathBuf::from(&home);

        if let Err(e) = std::fs::create_dir_all(&home_path) {
            let _ = self.bus.send_error(&format!(
                "Failed to create home directory '{}': {}",
                home, e
            ));
            return;
        }

        let session_id = Uuid::new_v4();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();

        let info = SessionInfo {
            id: session_id,
            name,
            home_folder: home,
            created_at: now,
        };

        let session = Session {
            info: info.clone(),
            workspace_ids: HashSet::new(),
        };

        self.sessions.insert(session_id, session);
        let _ = self.bus.send(&BusMessage::SessionCreated { session: info });
    }

    fn list_agent_types(&self) {
        let types = self.agent_registry.list_all();
        let _ = self.bus.send(&BusMessage::AgentTypes { types });
    }

    fn attach_session(&self, session_id: SessionId) {
        match self.sessions.get(&session_id) {
            Some(session) => {
                let workspaces: Vec<WorkspaceInfo> = session
                    .workspace_ids
                    .iter()
                    .filter_map(|wid| self.workspaces.get(wid).cloned())
                    .collect();
                let _ = self.bus.send(&BusMessage::SessionAttached {
                    session: session.info.clone(),
                    workspaces,
                });
            }
            None => {
                let _ = self
                    .bus
                    .send_error(&format!("Session {} not found", session_id));
            }
        }
    }

    fn detach_session(&self, session_id: SessionId) {
        if self.sessions.contains_key(&session_id) {
            let _ = self.bus.send_status(&format!("Detached from session {}", session_id));
        } else {
            let _ = self
                .bus
                .send_error(&format!("Session {} not found", session_id));
        }
    }

    // ── Workspace management ──────────────────────────────────────

    fn spawn_workspace(
        &mut self,
        session_id: SessionId,
        agent_type: String,
        folder: Option<String>,
    ) {
        // Validate session exists
        let session = match self.sessions.get(&session_id) {
            Some(s) => s,
            None => {
                let _ = self
                    .bus
                    .send_error(&format!("Session {} not found", session_id));
                return;
            }
        };

        // Validate agent type
        let agent_def = match self.agent_registry.get(&agent_type) {
            Some(a) => a.clone(),
            None => {
                let _ = self
                    .bus
                    .send_error(&format!("Unknown agent type: {}", agent_type));
                return;
            }
        };

        // Determine workspace folder
        let ws_folder = folder.unwrap_or_else(|| session.info.home_folder.clone());
        let ws_path = PathBuf::from(&ws_folder);

        if let Err(e) = std::fs::create_dir_all(&ws_path) {
            let _ = self.bus.send_error(&format!(
                "Failed to create workspace folder '{}': {}",
                ws_folder, e
            ));
            return;
        }

        // Create PTY
        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => {
                let _ = self
                    .bus
                    .send_error(&format!("Failed to create PTY: {}", e));
                return;
            }
        };

        // Build command
        let mut cmd = CommandBuilder::new(&agent_def.command);
        for arg in &agent_def.default_args {
            cmd.arg(arg);
        }
        cmd.cwd(&ws_path);

        // Spawn process
        let child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(e) => {
                let _ = self.bus.send_error(&format!(
                    "Failed to spawn '{}': {}",
                    agent_def.command, e
                ));
                return;
            }
        };

        // Get reader and writer from master
        let reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                let _ = self.bus.send_error(&format!("Failed to clone PTY reader: {}", e));
                return;
            }
        };

        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(e) => {
                let _ = self.bus.send_error(&format!("Failed to get PTY writer: {}", e));
                return;
            }
        };

        // Create workspace info
        let workspace_id = Uuid::new_v4();
        let ws_info = WorkspaceInfo {
            id: workspace_id,
            session_id,
            agent_type: agent_type.clone(),
            folder: ws_folder,
            cols: 80,
            rows: 24,
            running: true,
        };

        // Store workspace info
        self.workspaces.insert(workspace_id, ws_info.clone());
        self.sessions
            .get_mut(&session_id)
            .unwrap()
            .workspace_ids
            .insert(workspace_id);

        // Store I/O handles
        self.workspace_io.insert(
            workspace_id,
            WorkspaceIO {
                master: pair.master,
                writer,
                child,
            },
        );

        // Spawn reader thread
        let bus_clone = Arc::clone(&self.bus);
        let reader_workspace_id = workspace_id;
        std::thread::spawn(move || {
            read_pty_output(bus_clone, reader_workspace_id, reader);
        });

        let _ = self
            .bus
            .send(&BusMessage::WorkspaceSpawned { workspace: ws_info });
    }

    fn handle_terminal_input(&mut self, workspace_id: WorkspaceId, bytes: Vec<u8>) {
        if let Some(io) = self.workspace_io.get_mut(&workspace_id) {
            if let Err(e) = io.writer.write_all(&bytes) {
                let _ = self.bus.send_error(&format!(
                    "Failed to write to workspace {} PTY: {}",
                    workspace_id, e
                ));
            }
        } else {
            let _ = self.bus.send_error(&format!(
                "Workspace {} not found or not running",
                workspace_id
            ));
        }
    }

    fn handle_terminal_resize(&mut self, workspace_id: WorkspaceId, cols: u16, rows: u16) {
        if let Some(io) = self.workspace_io.get(&workspace_id) {
            if let Err(e) = io.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                let _ = self.bus.send_error(&format!(
                    "Failed to resize workspace {} PTY: {}",
                    workspace_id, e
                ));
            }
            // Update stored info
            if let Some(info) = self.workspaces.get_mut(&workspace_id) {
                info.cols = cols;
                info.rows = rows;
            }
        } else {
            let _ = self.bus.send_error(&format!(
                "Workspace {} not found or not running",
                workspace_id
            ));
        }
    }
}

/// PTY reader thread: reads output from the PTY and sends it through the bus.
/// Runs until EOF or error, then sends WorkspaceExited.
fn read_pty_output(bus: SharedBus, workspace_id: WorkspaceId, mut reader: Box<dyn Read + Send>) {
    let mut buf = [0u8; 8192];

    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                // EOF — process exited
                break;
            }
            Ok(n) => {
                let bytes = buf[..n].to_vec();
                let msg = BusMessage::TerminalOutput {
                    workspace_id,
                    bytes,
                };
                if bus.send(&msg).is_err() {
                    break; // Bus closed
                }
            }
            Err(_) => {
                break; // Read error
            }
        }
    }

    // Send exit event
    let _ = bus.send(&BusMessage::WorkspaceExited {
        workspace_id,
        code: -1,
    });
}
