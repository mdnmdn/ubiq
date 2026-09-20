//! How a project is worn by something that is not the project: its colour and the one or two
//! characters that stand for its name.
//!
//! Three surfaces say "this belongs to that project" in a space too small for a name — the rail's
//! badges, a Teams card under [`crate::state::teams::TeamsSpan::Window`], and the session pills
//! over it. They are three shapes around **one** resolution, which lives here so a project cannot
//! read as `AC` in the rail and `A` on a card.
//!
//! Nothing new in `theme.rs`: the tint is [`theme::project_tint`], the same call the mark, the
//! badge and the picker already make.

use gpui::{App, Rgba, SharedString};

use ubiq_proto::ids::ProjectId;

use crate::state::WindowRegistry;
use crate::theme;

/// A project at badge size: the tint it is known by, the characters that stand for it, and the
/// name they abbreviate — which is what every surface drawing one puts in its tooltip.
#[derive(Clone, Debug)]
pub struct ProjectFace {
    pub name: SharedString,
    pub initials: SharedString,
    pub tint: Rgba,
}

/// What a project looks like at that size, or `None` when the registry has never heard of it.
///
/// **An override wins outright** — 1 or 2 characters, shown as the project settings field holds
/// them. Empty is "no override", which is the name's own first letter, the way it always read.
pub fn project_face(id: ProjectId, cx: &App) -> Option<ProjectFace> {
    let project = WindowRegistry::read(cx).project(id)?;
    let record = &project.record;
    let initials = if record.initials.is_empty() {
        record
            .name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string()
    } else {
        record.initials.clone()
    };
    Some(ProjectFace {
        name: SharedString::from(record.name.clone()),
        initials: SharedString::from(initials),
        tint: theme::project_tint(record.temporary, record.colour, record.custom_colour),
    })
}
