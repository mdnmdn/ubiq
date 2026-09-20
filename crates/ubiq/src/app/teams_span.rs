//! The one switch behind the Teams toolbar's span control: whether the canvas is about the active
//! project or about every project the window holds.
//!
//! A file of its own rather than a corner of [`super::teams`], because the span is the window's
//! own fact — like the zoom and the arrangement — and every reader of it is already served by the
//! accessors in [`super::shell`]. Nothing here touches a view: switching the span switches which
//! [`crate::state::teams::TeamsView`] those accessors hand back, and each span keeps its own
//! arrangement over its own set of cards.

use super::*;

impl AppState {
    /// Flip the span. The selection, the filters and the zoom are not touched: they belong to
    /// whichever view the new span points at, and the one being left keeps its own.
    pub fn toggle_teams_span(&mut self, cx: &mut Context<Self>) {
        self.teams_span = match self.teams_span {
            TeamsSpan::Project => TeamsSpan::Window,
            TeamsSpan::Window => TeamsSpan::Project,
        };
        cx.notify();
    }

    /// Whether there is anything on the other side of that switch — more than one project this
    /// window both holds and has built state for.
    ///
    /// **Not [`Self::teams_projects`]**, which answers for the span that is *up*: under
    /// [`TeamsSpan::Project`] that is always the active project alone, so a control asking it
    /// would hide itself and never come back. This is the window span's own arithmetic, asked
    /// whatever the span is.
    pub fn teams_span_choice(&self, cx: &App) -> bool {
        self.window_projects(cx).len() > 1
    }
}
