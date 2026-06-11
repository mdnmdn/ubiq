use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;
use uuid::Uuid;

// Coordinator module (placeholder)
mod coordinator;
use coordinator::Coordinator;

// Pane ID type
type PaneId = Uuid;

// Message types for the transport contract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CoordinatorMessage {
    // Downstream (coordinator -> UI)
    Output { pane_id: PaneId, bytes: Vec<u8> },
    Exited { pane_id: PaneId, code: i32 },
    
    // Upstream (UI -> coordinator)
    Input { pane_id: PaneId, bytes: Vec<u8> },
    
    // Control (bidirectional)
    Spawn { pane_id: PaneId, harness: String, args: Vec<String> },
    Resize { pane_id: PaneId, cols: u16, rows: u16 },
    Focus { pane_id: PaneId },
}

// Application state
pub struct AppState {
    coordinator: Mutex<Coordinator>,
    panes: Mutex<HashMap<PaneId, PaneState>>,
}

// Pane state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneState {
    pub id: PaneId,
    pub harness: Option<String>,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub focused: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            coordinator: Mutex::new(Coordinator::new()),
            panes: Mutex::new(HashMap::new()),
        }
    }
}

// Tauri commands

#[tauri::command]
fn spawn_pane(
    state: State<AppState>,
    harness: String,
    args: Vec<String>,
) -> Result<PaneId, String> {
    let pane_id = Uuid::new_v4();
    let mut coordinator = state.coordinator.lock().map_err(|e| e.to_string())?;
    let mut panes = state.panes.lock().map_err(|e| e.to_string())?;
    
    // Spawn the harness in a PTY
    coordinator.spawn_harness(pane_id, &harness, &args)
        .map_err(|e| e.to_string())?;
    
    // Store pane state
    let pane_state = PaneState {
        id: pane_id,
        harness: Some(harness),
        args,
        cols: 80,
        rows: 24,
        focused: false,
    };
    panes.insert(pane_id, pane_state);
    
    Ok(pane_id)
}

#[tauri::command]
fn send_input(
    state: State<AppState>,
    pane_id: PaneId,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let coordinator = state.coordinator.lock().map_err(|e| e.to_string())?;
    coordinator.send_input(pane_id, &bytes)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn resize_pane(
    state: State<AppState>,
    pane_id: PaneId,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let coordinator = state.coordinator.lock().map_err(|e| e.to_string())?;
    let mut panes = state.panes.lock().map_err(|e| e.to_string())?;
    
    // Resize the PTY
    coordinator.resize_pty(pane_id, cols, rows)
        .map_err(|e| e.to_string())?;
    
    // Update pane state
    if let Some(pane) = panes.get_mut(&pane_id) {
        pane.cols = cols;
        pane.rows = rows;
    }
    
    Ok(())
}

#[tauri::command]
fn close_pane(
    state: State<AppState>,
    pane_id: PaneId,
) -> Result<(), String> {
    let mut coordinator = state.coordinator.lock().map_err(|e| e.to_string())?;
    let mut panes = state.panes.lock().map_err(|e| e.to_string())?;
    
    // Close the PTY
    coordinator.close_pty(pane_id)
        .map_err(|e| e.to_string())?;
    
    // Remove pane state
    panes.remove(&pane_id);
    
    Ok(())
}

#[tauri::command]
fn get_panes(
    state: State<AppState>,
) -> Result<Vec<PaneState>, String> {
    let panes = state.panes.lock().map_err(|e| e.to_string())?;
    Ok(panes.values().cloned().collect())
}

#[tauri::command]
fn focus_pane(
    state: State<AppState>,
    pane_id: PaneId,
) -> Result<(), String> {
    let mut panes = state.panes.lock().map_err(|e| e.to_string())?;
    
    // Update focus state
    for pane in panes.values_mut() {
        pane.focused = pane.id == pane_id;
    }
    
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            spawn_pane,
            send_input,
            resize_pane,
            close_pane,
            get_panes,
            focus_pane,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}