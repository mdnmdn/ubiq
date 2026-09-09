//! Application state: what the workbench knows, with no opinion about how it is drawn.
//!
//! Nothing in here renders and nothing in here names a process, a path on disk or a file
//! descriptor. The UI reads these types. Projects and the work arrive from the host as
//! projections, and so does the Git screen's refs and history.
//!
//! The work's own records are not re-exported through here. They are `ubiq_proto::work`'s, and
//! naming them at each use site is what keeps the dependency direction visible: a re-export would
//! let `ui/` draw the domain without ever mentioning the contract it came across.

pub mod agents;
pub mod board;
pub mod chat;
pub mod clone;
pub mod conversation;
pub mod diagrams;
pub mod dock;
pub mod editor;
pub mod explorer;
pub mod file_picker;
pub mod git;
pub mod image_edit;
pub mod layout;
pub mod logs;
pub mod nav;
pub mod navigator;
pub mod orchestration;
pub mod prefs;
pub mod remote;
pub mod remote_hosts;
pub mod scene;
pub mod search;
pub mod settings;
pub mod sink;
pub mod stats;
pub mod viewport;
pub mod vim;
pub mod when;
pub mod windows;
pub mod work;
pub mod workbench;

pub use agents::{AgentsView, BenchRow, Column};
pub use board::{BoardState, Field, TaskForm};
pub use chat::{
    AttachChoices, ChatPick, ChatPicks, ChatTab, StartOffer, attach_choices, chat_picks,
};
pub use conversation::{ConvBlock, Conversation, Pending, Run};
pub use diagrams::{DiagramImage, DiagramPalette};
pub use dock::{ChatId, PanelClass, PanelKind, Region};
pub use editor::{EditorPaneState, FileBody, FileLanguage, OpenFile, SaveState};
pub use explorer::{
    ExplorerAction, ExplorerEntry, ExplorerKey, ExplorerMenu, ExplorerPressed, ExplorerState,
    ExplorerView, FileNode, Follow, GitStatus, NodeKind, Row, Toggle,
};
pub use file_picker::{
    Commit, FilePickerState, PickKind, PickerCount, PickerNode, PickerOwner, PickerRequest,
    PickerRow, PickerView,
};
pub use git::{CommitRow, GitView, RefRow, RefSection, Side};
pub use image_edit::{EditOp, ImageEdit, ImageTool, ShapeKind};
pub use layout::Layout;
pub use logs::LogState;
pub use nav::{Destination, Locus, View};
pub use orchestration::{Carry, Grain, GraphView, Held, InspectorTab, Selection};
pub use remote::{AttemptId, RemoteConnectState, RemoteConnectStep};
pub use remote_hosts::{RemoteManagerState, TestOutcome};
pub use scene::{Element, ElementKind, Rgba8, Scene, SceneError};
pub use search::{ActiveSearch, SearchState};
pub use sink::{SinkDoc, SinkModal, SinkSection, SinkState};
pub use windows::{ProjectGroups, WindowRegistry, WindowSlot};
pub use work::WorkProjection;
pub use workbench::{
    FileDialog, HarnessChoice, MenuId, NewPaneRow, ProjectSettings, ProjectSettingsMode, RailMode,
    RowAction, WorkbenchState,
};
