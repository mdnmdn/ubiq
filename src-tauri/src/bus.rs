use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use crate::messages::BusMessage;

/// The Rust-side bus. Wraps Tauri's AppHandle for message dispatch.
///
/// Outbound (Rust → JS): `send()` emits a Tauri event.
/// Inbound (JS → Rust):  the JS side calls the `bus_command` Tauri command,
///                        which calls `orchestrator.handle_message()`.
pub struct Bus {
    app: AppHandle,
}

impl Bus {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// Send a message from Rust to the JS frontend.
    /// Serializes the message to JSON and emits it as a Tauri event.
    pub fn send(&self, msg: &BusMessage) -> Result<(), String> {
        let json = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        self.app
            .emit("bus:message", json)
            .map_err(|e| e.to_string())
    }

    /// Convenience: send an error message to the UI.
    pub fn send_error(&self, message: &str) -> Result<(), String> {
        self.send(&BusMessage::Error {
            message: message.to_string(),
        })
    }

    /// Convenience: send a status message to the UI.
    pub fn send_status(&self, message: &str) -> Result<(), String> {
        self.send(&BusMessage::Status {
            message: message.to_string(),
        })
    }

    pub fn app_handle(&self) -> &AppHandle {
        &self.app
    }
}

/// Shared bus reference used across the application.
pub type SharedBus = Arc<Bus>;
