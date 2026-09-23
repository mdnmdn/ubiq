//! What a Teams card's state looks like: the mark it wears and the chip beside it.
//!
//! **The vocabulary is [`crate::state::status`]'s and the colours are [`crate::ui::work`]'s.**
//! This module adds a *shape* — the hexagon — and a chip; it adds no state and no token. An agent
//! and a delegate go through the same two functions here, because since the dictionaries were
//! unified there is nothing left to tell them apart at this layer.
//!
//! **No glyph is invented at draw time.** The `pane` set was drawn for exactly these readings;
//! the rest are Lucide's, taken as they come.

use gpui::{
    ElementId, InteractiveElement as _, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement as _, Styled, div, px,
};
use gpui_component::{Sizable as _, tooltip::Tooltip};

use crate::state::status::{Doing, Lifecycle, Status};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{hex_mark, mono, pill};
use crate::ui::project_face::ProjectFace;
use crate::ui::work::{doing_colour, lifecycle_colour, status_colour, status_icon};

/// The hexagonal status mark — the one mark an agent card and a delegate card both wear, top-left,
/// before the name.
///
/// **Two readings in one glyph.** The transparent outer border is the lifecycle: whether this
/// execution can still do anything. The inner fill is the activity, or — once the lifecycle is
/// `Ended` — the result it came back with. A delegate that finished with an error keeps its danger
/// core, because that is still the one thing worth an eye; a delegate that came back clean is grey
/// through and through — it is not working any more, and a green core past that fact would read as
/// still going. An execution whose activity nothing reports is the outline alone.
///
/// **The core pulses while there is somewhere to look.** Only [`Lifecycle::Working`] does — not
/// idle, not done, not waiting on the reader, the three restful readings a moving core would cry
/// wolf over. `id` names the animation, so two marks on the same canvas never share a clock.
pub fn status_mark(status: Status, side: f32, id: impl Into<ElementId>) -> impl IntoElement {
    let fill = match status.doing {
        Doing::Unknown => None,
        Doing::Done => Some(theme::text_faint()),
        doing => Some(doing_colour(doing)),
    };
    let pulse = status.lifecycle == Lifecycle::Working;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(hex_mark(
            id,
            lifecycle_colour(status.lifecycle),
            fill,
            side,
            pulse,
        ))
}

/// The colour a Teams card reads for its state, past [`status_mark`]'s own rule: gray once the
/// card is [`Doing::Done`].
///
/// **The mark already draws it this way** — `Done`'s fill is [`theme::text_faint`], not the
/// success green [`status_colour`] answers for it, because a finished delegate is spent rather
/// than a passing check. [`status_chip`] and a card's own edge read the same [`Status`] the mark
/// does, so they take the same answer here rather than disagreeing about what "done" looks like
/// on the one surface that draws all three (`T-106`).
pub fn card_colour(status: Status) -> Rgba {
    match status.doing {
        Doing::Done => theme::text_faint(),
        _ => status_colour(status),
    }
}

/// The chip a card carries: the state's glyph, then the word for it.
///
/// [`crate::ui::kit::state_chip`] with a shape in place of its dot — colour and wording together
/// is the rule, and the glyph is the third reading, for the states that share a colour. The chip
/// says the *compact* reading (`done`, `tools`, `needs you`); the precise pair is the card's
/// tooltip, which is what [`Status::label`] is for.
pub fn status_chip(status: Status, zoom: f32) -> impl IntoElement {
    let colour = card_colour(status);
    pill(colour)
        .h(px(22. * zoom))
        .px(px(6. * zoom))
        .gap(px(5. * zoom))
        .children(status_icon(status).map(|icon| {
            icon.with_size(theme::icon_sm() * zoom)
                .flex_none()
                .text_color(colour)
        }))
        .child(
            mono(status.chip(), theme::text())
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

// A delegate has no presentation of its own any more. It reads the same [`Status`] an agent does,
// through [`status_mark`] and [`status_chip`] above — which is the whole point of unifying the
// dictionaries, and the reason `delegate_icon`, `delegate_colour`, `delegate_mark` and
// `delegate_chip` are gone rather than kept as aliases.
