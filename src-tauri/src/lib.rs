mod agent;
mod bus;
mod messages;
mod orchestrator;

use std::sync::Mutex;
use tauri::Manager;
use tauri::State;

use bus::{Bus, SharedBus};
use messages::BusMessage;
use orchestrator::Orchestrator;

pub struct AppState {
    bus: SharedBus,
    orchestrator: Mutex<Orchestrator>,
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

            app.manage(AppState {
                bus,
                orchestrator: Mutex::new(orchestrator),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![bus_command])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
