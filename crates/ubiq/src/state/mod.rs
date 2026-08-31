//! Application state: what the workbench knows, with no opinion about how it is drawn.
//!
//! Nothing in here renders and nothing in here names a process, a path on disk or a file
//! descriptor. The UI reads these types. Projects arrive from the host as a projection; the rest
//! is still seeded from `sample.rs`.

pub mod chat;
pub mod editor;
pub mod explorer;
pub mod logs;
pub mod prefs;
pub mod sample;
pub mod when;
pub mod windows;
pub mod workbench;

pub use chat::{
    Block, Chat, ChatMessage, ChatState, DiffKind, DiffLine, HARNESSES, MODELS, MODES, RunState,
    THINKING, ToolCall, ToolKind,
};
pub use editor::{EditorPaneState, FileLanguage, OpenFile};
pub use explorer::{ExplorerState, FileNode, GitStatus, NodeKind, Row};
pub use logs::LogState;
pub use windows::{ProjectGroups, WindowRegistry, WindowSlot};
pub use workbench::{MenuId, RailMode, RowAction, WorkbenchState};
