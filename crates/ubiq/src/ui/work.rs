//! What a work record reads as: the colour an activity or a bucket takes, and the glyph a role
//! wears.
//!
//! Three screens and the status bar all draw the same records — the agents screen, the
//! orchestration graph, the tasks board — and none of them may invent a colour for a state the
//! others already have one for. So the mapping from a state to a token lives here, once, and
//! `ubiq_proto::work` keeps the words while `crate::theme` keeps the values.

use gpui::{IntoElement, ParentElement, Rgba, Styled, div, px};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::{Activity, Bucket};

use crate::theme;

/// What an activity reads as. The four buckets share the four status tokens, and the three ways of
/// working share the one that means "moving", so no screen asks the user to learn a colour that
/// means nothing anywhere else in the window.
pub fn activity_colour(activity: Activity) -> Rgba {
    bucket_colour(activity.bucket())
}

pub fn bucket_colour(bucket: Bucket) -> Rgba {
    match bucket {
        Bucket::Running => theme::success(),
        Bucket::Waiting => theme::info(),
        Bucket::Ended => theme::text_faint(),
        Bucket::Error => theme::danger(),
    }
}

/// The glyph a role wears. Ubiq ships no icon set, so a role borrows the nearest thing in the
/// component library's bundle.
pub fn role_icon(role: &str) -> IconName {
    match role.to_lowercase().as_str() {
        "project manager" | "activity coordinator" => IconName::Asterisk,
        "analyst" | "investigator" => IconName::Search,
        "verifier" => IconName::CircleCheck,
        "documentation" => IconName::BookOpen,
        _ => IconName::SquareTerminal,
    }
}

/// A role's glyph, at the size a card, a column header and the inspector all draw it.
pub fn role_mark(role: &str, colour: Rgba, side: f32) -> impl IntoElement {
    div()
        .size(px(side))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(theme::surface_raised())
        .child(
            Icon::new(role_icon(role))
                .with_size(Size::XSmall)
                .text_color(colour),
        )
}
