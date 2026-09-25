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

use crate::state::status::{Doing, Lifecycle, Status};
use crate::state::work::WorkState;
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

/// The worst thing happening in a group of executions: an error over a wait over movement over
/// nothing.
///
/// **One rule, two readers.** A folded session's left bar (`ui/agents/sidebar.rs`) and a mission's
/// hexagon (M6) both say "what would you want to be told first about everything in here", and a
/// rule read in two places is a rule that drifts — so it is answered here, once, beside the colour
/// it resolves to. An empty group answers [`Bucket::Ended`]: there is nothing happening in it.
pub fn worst_bucket(buckets: impl IntoIterator<Item = Bucket>) -> Bucket {
    let mut worst = Bucket::Ended;
    for bucket in buckets {
        match bucket {
            Bucket::Error => return Bucket::Error,
            Bucket::Waiting => worst = Bucket::Waiting,
            Bucket::Running if worst != Bucket::Waiting => worst = Bucket::Running,
            _ => {}
        }
    }
    worst
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

/// What a [`Lifecycle`] reads as — four colours, and only four.
///
/// **Yellow needs you, blue is working, green is alive and idle, grey is stopped.** A lifecycle is
/// read at a glance from across a window full of columns, so what it has to answer is "does this
/// one want me". `Working` is one blue rather than a palette per activity for the same reason:
/// *which kind* of work is the [`Doing`] half's question, and it is answered beside this, not
/// instead of it.
pub fn lifecycle_colour(state: Lifecycle) -> Rgba {
    match state {
        Lifecycle::Waiting => theme::warning(),
        Lifecycle::Working => theme::info(),
        Lifecycle::Ready | Lifecycle::Idle => theme::success(),
        Lifecycle::Starting | Lifecycle::Unloaded | Lifecycle::Ended => theme::text_faint(),
    }
}

/// What a [`Doing`] reads as. The three ways of working share one token, because they are one
/// answer to "is it moving"; a result takes the token the window already gives that outcome
/// everywhere else, so a delegate that came back is the same green as a passing check.
pub fn doing_colour(doing: Doing) -> Rgba {
    match doing {
        Doing::Thinking | Doing::Writing | Doing::Tools => theme::info(),
        Doing::NeedsYou => theme::warning(),
        Doing::Done => theme::success(),
        Doing::Failed => theme::danger(),
        Doing::Queued | Doing::Unknown => theme::text_faint(),
    }
}

/// The one colour a chip, a card's edge or a row takes for the pair: the activity's where there is
/// one, the lifecycle's where the activity says nothing. A surface with room for two marks draws
/// both halves — [`crate::ui::kit::hex_mark`] is the one that does — and a surface with room for
/// one draws this.
pub fn status_colour(status: Status) -> Rgba {
    match status.doing {
        Doing::Unknown => lifecycle_colour(status.lifecycle),
        doing => doing_colour(doing),
    }
}

/// What a [`WorkState`] reads as: the strong token a dot, a segment or a left edge takes, and the
/// soft one behind it (M26).
///
/// Two states have one colour rather than two — *in review* is `accent_muted` and *abandoned* is
/// `text_muted`, both already the muted end of their own pair — so the soft half repeats them.
/// Nothing here is new: every token below already has a value in both palettes.
pub fn work_state_colour(state: WorkState) -> Rgba {
    match state {
        WorkState::Blocked => theme::danger(),
        WorkState::Waiting => theme::warning(),
        WorkState::Ready => theme::info(),
        WorkState::InProgress => theme::accent(),
        WorkState::InReview => theme::accent_muted(),
        WorkState::Done => theme::success(),
        WorkState::Abandoned => theme::text_muted(),
    }
}

/// The soft half of the pair above — the fill under a chip, a tint on a row.
pub fn work_state_soft(state: WorkState) -> Rgba {
    match state {
        WorkState::Blocked => theme::danger_soft(),
        WorkState::Waiting => theme::warning_soft(),
        WorkState::Ready => theme::info_soft(),
        WorkState::InProgress => theme::accent_soft(),
        WorkState::InReview => theme::accent_muted(),
        WorkState::Done => theme::success_soft(),
        WorkState::Abandoned => theme::text_muted(),
    }
}

/// The glyph an activity or a result wears. `None` is [`Doing::Unknown`]: an activity nothing
/// reports is drawn as nothing rather than as a guess.
pub fn doing_icon(doing: Doing) -> Option<Icon> {
    Some(match doing {
        Doing::Queued => Icon::new(IconName::Pause),
        // The harness is working with nothing measurable to report — the goal `pane-thinking` was
        // drawn for.
        Doing::Thinking => Icon::new(UbiqIcon::PaneThinking),
        Doing::Writing => Icon::new(UbiqIcon::PaneWriting),
        Doing::Tools => Icon::new(UbiqIcon::PaneTools),
        // The one state a reader has to act on, and the one mark the window already uses for it.
        Doing::NeedsYou => Icon::new(UbiqIcon::PaneAwaiting),
        Doing::Done => Icon::new(IconName::CircleCheck),
        Doing::Failed => Icon::new(IconName::TriangleAlert),
        Doing::Unknown => return None,
    })
}

/// The glyph a lifecycle wears where the activity has none to lend.
pub fn lifecycle_icon(state: Lifecycle) -> Option<Icon> {
    Some(match state {
        Lifecycle::Waiting => Icon::new(UbiqIcon::PaneAwaiting),
        Lifecycle::Working => Icon::new(UbiqIcon::PaneThinking),
        Lifecycle::Unloaded => Icon::new(UbiqIcon::PaneUnloaded),
        // The same mark the conversation header draws for a harness that has gone.
        Lifecycle::Ended => Icon::new(IconName::CircleX),
        Lifecycle::Idle | Lifecycle::Ready => Icon::new(IconName::Pause),
        Lifecycle::Starting => return None,
    })
}

/// The pair's glyph: the activity's, falling back to the lifecycle's. `None` only where neither
/// half has anything to say.
pub fn status_icon(status: Status) -> Option<Icon> {
    doing_icon(status.doing).or_else(|| lifecycle_icon(status.lifecycle))
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
