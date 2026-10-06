//! The Archify viewer's interface half: a contributed viewer of the IDE editor (`viewer`, `D204`)
//! registered by `ext::viewer::base_registry`, and its settings section. The IDE owns the file,
//! this module draws the picture; the per-tab state is `state::archify`. Nothing here touches a
//! file, a process or a socket.

pub mod agent;
pub mod finder;
pub mod lens;
pub mod paint;
pub mod panels;
pub mod settings;
pub mod theme;
pub mod viewer;
