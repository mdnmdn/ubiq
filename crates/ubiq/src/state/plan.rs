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
//! **The second kind of annotated document is an ordinary markdown file in the project's own
//! tree** — [`file_document`]. It is the same family on the wire, the same block matcher and the
//! same orphaning rule host-side; what differs is only where the body and its sidecar sit, and the
//! host answers that from the handle. A plan's annotations also reach the agents through
//! `ubiq-plan`; a file's simply sit there, until a later card gives them a reader.

use ubiq_proto::ids::{ProjectId, TaskId};

use crate::state::document::DocumentHandle;

/// A task's plan, as a document handle.
pub fn plan_document(project_id: ProjectId, task_id: TaskId) -> DocumentHandle {
    DocumentHandle::Plan {
        project_id,
        task_id,
    }
}

/// An ordinary markdown file in the project's working tree, as a document handle.
///
/// `rel_path` is project-relative — the interface never holds an absolute one — and the host
/// refuses a path that is not markdown or does not land inside the project.
pub fn file_document(project_id: ProjectId, rel_path: impl Into<String>) -> DocumentHandle {
    DocumentHandle::File {
        project_id,
        rel_path: rel_path.into(),
    }
}
