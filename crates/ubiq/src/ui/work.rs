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
use crate::ui::kit::UbiqIcon;

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

/// The glyph a role wears. Four names carry a shape of their own — drawn rather than borrowed, so
/// nothing generic reads as Claude's asterisk — and everything else falls through to the honest
/// fallback.
pub fn role_icon(role: &str) -> Icon {
    match role.to_lowercase().as_str() {
        "project manager" | "activity coordinator" => Icon::new(UbiqIcon::RoleManager),
        "analyst" | "investigator" => Icon::new(UbiqIcon::RoleAnalyst),
        "verifier" => Icon::new(UbiqIcon::RoleVerifier),
        "documentation" => Icon::new(IconName::BookOpen),
        _ => Icon::new(UbiqIcon::RoleWorker),
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
        .child(role_icon(role).with_size(Size::XSmall).text_color(colour))
}
