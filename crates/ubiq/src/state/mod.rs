//! Application state: what the workbench knows, with no opinion about how it is drawn.
//!
//! Nothing in here renders and nothing in here names a process, a path on disk or a file
//! descriptor. The UI reads these types; `sample.rs` seeds them until a coordinator exists.

pub mod chat;
pub mod editor;
pub mod explorer;
pub mod sample;
pub mod workbench;

pub use chat::{
    Block, Chat, ChatMessage, ChatState, DiffKind, DiffLine, HARNESSES, MODELS, MODES, RunState,
    THINKING, ToolCall, ToolKind,
};
pub use editor::{EditorPaneState, FileLanguage, OpenFile};
pub use explorer::{ExplorerState, FileNode, GitStatus, NodeKind, Row};
pub use workbench::{MenuId, Project, RailMode, WorkbenchState};
