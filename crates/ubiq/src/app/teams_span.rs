//! What the Teams canvas is about — the active project, or every project the window holds — read
//! off the rail rather than stored.
//!
//! There are two rail entries over the one screen: `Teams` in the PROJECT group and `TeamsAll` in
//! the APP group, so the span is already written down as "where the window is", and a second copy
//! of it on [`AppState`] could only ever disagree with the rail. A file of its own rather than a
//! corner of [`super::teams`], because every reader of the span is one of the accessors in
//! [`super::shell`] and this is the one line they all ask. Nothing here touches a view: the span
//! decides which [`crate::state::teams::TeamsView`] those accessors hand back, and each span keeps
//! its own arrangement over its own set of cards.

use super::*;

impl AppState {
    /// Which span the canvas is drawing, which is only ever a question about the rail.
    pub fn teams_span(&self) -> TeamsSpan {
        match self.workbench.rail_mode {
            RailMode::TEAMS_ALL => TeamsSpan::Window,
            _ => TeamsSpan::Project,
        }
    }

    /// Whether a start raised from the canvas has to ask which project it is for: the canvas
    /// spans the window, and the window holds more than one project to choose between.
    ///
    /// **Not [`Self::teams_projects`]**, which answers with the projects the span is actually
    /// drawing — under [`TeamsSpan::Project`] that is always the active project alone, so both
    /// halves of this are needed and neither implies the other.
    pub fn teams_project_choice(&self, cx: &App) -> bool {
        self.teams_span() == TeamsSpan::Window && self.window_projects(cx).len() > 1
    }
}
