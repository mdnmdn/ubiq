//! The plan as a document: the one implementation of [`crate::state::document::DocumentHandle`].
//!
//! **Everything about the surface itself moved to [`crate::state::document`] in slice 6.** What
//! is left here is what is true of a *plan* and of nothing else: it belongs to a task carrying a
//! `level`, it is stored beside that project's tasks in the config root, and the plan family on
//! the wire is how it is read, written and annotated. The editor, the decorations, the thread
//! popover and the `/` menu never name any of that — they read a `DocumentEditor`.
//!
//! **The plan is no longer read-only.** Slice 3 put [`ubiq_proto::messages::Message::SavePlan`] on
//! the wire with no caller; `crate::app::plan::save_document` is that caller. An annotation is
//! still a comment rather than an edit, and the two travel separately: saving the body re-indexes
//! the blocks host-side, and a thread whose block vanished is orphaned rather than dropped.
//!
//! A second kind of annotated document is a storage decision before it is code — see
//! [`crate::state::document`]'s module documentation for the three things it would have to
//! supply.

use ubiq_proto::ids::{ProjectId, TaskId};

use crate::state::document::DocumentHandle;

/// A task's plan, as a document handle.
pub fn plan_document(project_id: ProjectId, task_id: TaskId) -> DocumentHandle {
    DocumentHandle::Plan {
        project_id,
        task_id,
    }
}
