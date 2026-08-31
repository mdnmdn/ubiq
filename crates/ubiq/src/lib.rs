// Ubiq: GPUI-based agent harness multiplexer
//
// Module structure:
// - app: AppState and main Render impl
// - orchestrator: Process and PTY lifecycle management
// - agent: Agent harness definitions and traits
// - pty: PTY I/O handling and stream management
// - ui: UI components and layout
// - state: State management and event handling
// - messages: Transport contract messages
// - mcp_server: Model Context Protocol server support
// - theme: Color tokens and theme definitions

pub mod app;
pub mod orchestrator;
pub mod agent;
pub mod pty;
pub mod ui;
pub mod state;
pub mod messages;
pub mod mcp_server;
pub mod theme;

pub use app::AppState;
pub use theme::Theme;
