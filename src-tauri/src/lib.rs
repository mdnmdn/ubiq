mod agent;
mod bus;
mod messages;
mod mcp_server;
mod orchestrator;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri::State;

use bus::{Bus, SharedBus};
use messages::BusMessage;
use mcp_server::DEFAULT_MCP_PORT;
use orchestrator::Orchestrator;

pub struct AppState {
    bus: SharedBus,
    orchestrator: Arc<Mutex<Orchestrator>>,
}

#[tauri::command]
fn bus_command(state: State<AppState>, message: String) -> Result<(), String> {
    let msg: BusMessage =
        serde_json::from_str(&message).map_err(|e| format!("Invalid message: {}", e))?;

    let mut orchestrator = state.orchestrator.lock().map_err(|e| e.to_string())?;
    orchestrator.handle_message(msg);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing (respects RUST_LOG env var).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let bus = std::sync::Arc::new(Bus::new(app.handle().clone()));

            let agents_path = app.path().resource_dir()?.join("agents.toml");
            let agent_registry = agent::AgentRegistry::load(agents_path).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to load agents.toml: {}", e);
                agent::AgentRegistry::empty()
            });

            let orchestrator = Orchestrator::new(bus.clone(), agent_registry);
            let orchestrator_arc = std::sync::Arc::new(Mutex::new(orchestrator));

            // Start the MCP server on a background thread.
            let mcp_addr: SocketAddr = ([127, 0, 0, 1], DEFAULT_MCP_PORT).into();
            mcp_server::start_mcp_server(mcp_addr, orchestrator_arc.clone());

            app.manage(AppState {
                bus,
                orchestrator: orchestrator_arc,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![bus_command])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
