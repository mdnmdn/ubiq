//! What a Teams card's state looks like: the colour it takes and the glyph it wears.
//!
//! **The colour is [`crate::ui::work`]'s, unchanged.** A state reads in its bucket's token here
//! exactly as it does on the agents screen, the board and the status bar — this module adds a
//! shape beside it, it does not add a vocabulary. Six states share four colours, and the glyph is
//! what tells `Thinking` from `Tools` and `Idle` from `Needs you`.
//!
//! **No glyph is invented at draw time.** Six come from the registry's `pane` set, which was
//! drawn for exactly these readings; the rest are Lucide's, taken as they come.

use gpui::{
    ElementId, InteractiveElement as _, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement as _, Styled, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, tooltip::Tooltip};

use crate::state::teams::{AgentStatus, DelegateStatus};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{UbiqIcon, mono, pill};
use crate::ui::project_face::ProjectFace;
use crate::ui::work::bucket_colour;

/// The glyph a card's state wears.
pub fn status_icon(status: AgentStatus) -> Icon {
    match status {
        // The harness is working with nothing measurable to report — the goal `pane-thinking` was
        // drawn for.
        AgentStatus::Thinking => Icon::new(UbiqIcon::PaneThinking),
        AgentStatus::Writing => Icon::new(UbiqIcon::PaneWriting),
        AgentStatus::Tools => Icon::new(UbiqIcon::PaneTools),
        // The one state a reader has to act on, and the one mark the window already uses for it.
        AgentStatus::NeedsYou => Icon::new(UbiqIcon::PaneAwaiting),
        AgentStatus::Idle => Icon::new(IconName::Pause),
        // The same mark the conversation header draws for a harness that has gone.
        AgentStatus::Ended => Icon::new(IconName::CircleX),
        AgentStatus::Failed => Icon::new(IconName::TriangleAlert),
    }
}

pub fn status_colour(status: AgentStatus) -> Rgba {
    bucket_colour(status.bucket())
}

/// The chip a card carries: the state's glyph, then the word for it.
///
/// [`crate::ui::kit::state_chip`] with a shape in place of its dot — colour and wording together
/// is the rule, and the glyph is the third reading, for the states that share a colour.
pub fn status_chip(status: AgentStatus, zoom: f32) -> impl IntoElement {
    let colour = status_colour(status);
    pill(colour)
        .h(px(22. * zoom))
        .px(px(6. * zoom))
        .gap(px(5. * zoom))
        .child(
            status_icon(status)
                .with_size(theme::icon_sm() * zoom)
                .flex_none()
                .text_color(colour),
        )
        .child(
            mono(status.label(), theme::text())
                .text_size(theme::font(Family::Chrome, Role::Meta) * zoom),
        )
}

/// The chip that says whose project this is: the project's swatch and its initials.
///
/// **Drawn only under [`crate::state::teams::TeamsSpan::Window`]**, beside [`status_chip`] on a
/// card and before the name on a session pill. A canvas about one project does not need to say
/// which, and two projects naming a session the same thing is exactly what this tells apart.
///
/// The swatch is the chip's own left edge — a coloured left edge is how a surface says what it
/// belongs to everywhere else in the window, and a second square beside it in the same colour
/// would be the same fact drawn twice. The initials are an abbreviation, so the name is the
/// tooltip, which is why this takes an id.
pub fn project_chip(id: impl Into<ElementId>, face: &ProjectFace, zoom: f32) -> impl IntoElement {
    let name = face.name.clone();
    pill(face.tint)
        .h(px(22. * zoom))
        .px(px(6. * zoom))
        .gap(px(5. * zoom))
        .id(id)
        .child(
            mono(face.initials.clone(), theme::text_muted())
                .text_size(theme::font(Family::Chrome, Role::Meta) * zoom),
        )
        .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
}

/// The glyph a delegate's box wears, and the colour it takes.
///
/// `None` is [`DelegateStatus::Unknown`] — a spawning call the transcript does not hold, which the
/// box draws as no mark rather than as a guess.
pub fn delegate_icon(status: DelegateStatus) -> Option<Icon> {
    Some(match status {
        DelegateStatus::Queued => Icon::new(IconName::Pause),
        DelegateStatus::Working => Icon::new(UbiqIcon::PaneThinking),
        DelegateStatus::NeedsYou => Icon::new(UbiqIcon::PaneAwaiting),
        DelegateStatus::Done => Icon::new(IconName::CircleCheck),
        DelegateStatus::Failed => Icon::new(IconName::TriangleAlert),
        DelegateStatus::Unknown => return None,
    })
}

pub fn delegate_colour(status: DelegateStatus) -> Rgba {
    bucket_colour(status.bucket())
}

/// The one mark a delegate's box has room for. About sixty pixels wide holds a glyph and a name,
/// so the word for the state goes in the box's tooltip and this is the glyph.
pub fn delegate_mark(status: DelegateStatus, zoom: f32) -> Option<impl IntoElement> {
    let icon = delegate_icon(status)?;
    Some(
        div().flex().flex_none().child(
            icon.with_size(theme::icon_sm() * zoom)
                .flex_none()
                .text_color(delegate_colour(status)),
        ),
    )
}

/// The chip a delegate card carries, at the full size the card now draws at: the state's glyph,
/// then the word for it. [`status_chip`] at a delegate's grain — a delegate is another agent doing
/// another piece of the work, and the one thing that told the two apart used to be that only the
/// agent card wore a chip at all.
pub fn delegate_chip(status: DelegateStatus, zoom: f32) -> impl IntoElement {
    let colour = delegate_colour(status);
    pill(colour)
        .h(px(22. * zoom))
        .px(px(6. * zoom))
        .gap(px(5. * zoom))
        .children(delegate_icon(status).map(|icon| {
            icon.with_size(theme::icon_sm() * zoom)
                .flex_none()
                .text_color(colour)
        }))
        .child(
            mono(status.label(), theme::text())
                .text_size(theme::font(Family::Chrome, Role::Meta) * zoom),
        )
}
