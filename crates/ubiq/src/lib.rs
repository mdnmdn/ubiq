// Ubiq: GPUI-based agent harness multiplexer
//
// Module structure:
// - app: AppState and main Render impl
// - ui: UI components and layout
// - state: State management and event handling
// - theme: Color tokens and theme definitions
// - ext: the extension spine — SlotId and the generic Registry<T> (D177, D178)

pub mod app;
pub mod ext;
pub mod state;
pub mod theme;
pub mod ui;
pub mod version;
pub mod web_export;

pub use app::AppState;
pub use theme::Theme;
