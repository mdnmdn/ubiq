//! Application state: what the workbench knows, with no opinion about how it is drawn.
//!
//! Nothing in here renders and nothing in here names a process, a path on disk or a file
//! descriptor. The UI reads these types. Projects and the work arrive from the host as
//! projections; the chat is the last thing still seeded from `sample.rs`.
//!
//! The work's own records are not re-exported through here. They are `ubiq_proto::work`'s, and
//! naming them at each use site is what keeps the dependency direction visible: a re-export would
//! let `ui/` draw the domain without ever mentioning the contract it came across.

pub mod agents;
pub mod board;
pub mod chat;
pub mod diagrams;
pub mod dock;
pub mod editor;
pub mod explorer;
pub mod file_picker;
pub mod layout;
pub mod logs;
pub mod prefs;
pub mod sample;
pub mod scene;
pub mod sink;
pub mod viewport;
pub mod when;
pub mod windows;
pub mod work;
pub mod workbench;

pub use agents::{Carry, Grain, GraphView, Held, InspectorTab, Selection};
pub use board::{BoardState, Field, TaskForm};
pub use chat::{
    Block, Chat, ChatMessage, ChatState, DiffKind, DiffLine, HARNESSES, MODELS, MODES, RunState,
    THINKING, ToolCall, ToolKind,
};
pub use diagrams::{DiagramImage, DiagramPalette};
pub use dock::{PanelClass, PanelKind, Region};
pub use editor::{EditorPaneState, FileBody, FileLanguage, OpenFile, SaveState};
pub use explorer::{ExplorerState, FileNode, GitStatus, NodeKind, Row, Toggle};
pub use file_picker::{
    Commit, FilePickerState, PickKind, PickerCount, PickerNode, PickerOwner, PickerRequest,
    PickerRow, PickerView,
};
pub use layout::Layout;
pub use logs::LogState;
pub use scene::{Element, ElementKind, Rgba8, Scene, SceneError};
pub use sink::{SinkDoc, SinkModal, SinkSection, SinkState};
pub use windows::{ProjectGroups, WindowRegistry, WindowSlot};
pub use work::WorkProjection;
pub use workbench::{MenuId, RailMode, RowAction, WorkbenchState};
