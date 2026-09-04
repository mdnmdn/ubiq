// Ubiq: GPUI-based agent harness multiplexer
//
// Module structure:
// - app: AppState and main Render impl
// - ui: UI components and layout
// - state: State management and event handling
// - theme: Color tokens and theme definitions

pub mod app;
pub mod state;
pub mod theme;
pub mod ui;
pub mod version;
pub mod web_export;

pub use app::AppState;
pub use theme::Theme;
