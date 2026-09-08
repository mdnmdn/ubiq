//! The bell's list: what has arrived, what is silenced, and the picker that silences more.
//!
//! One modal, in the kit's shape, over the window. Every control in it is a message to the host —
//! nothing here edits the list it is drawing, because the host owns it and every window redraws
//! from the same broadcast.
//!
//! **The origin is the addressing scheme, so the list draws it.** A row says which family raised
//! it, which actor inside that family and what kind of event, because those three are exactly
//! what a mute rule matches on: the mute control on a row offers the narrowest of them the row
//! actually carries, so silencing "this agent" is one click from the thing that was too loud.

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Rgba, StatefulInteractiveElement, Styled,
    Window, div, px, relative,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use ubiq_proto::notifications::{Level, MuteFor, MuteRule, MuteScope, Notification};

use crate::app::AppState;
use crate::state::when;
use crate::theme;
use crate::ui::kit::{
    card, choice_pill, ghost_button, icon_button, modal, mono, pill, section_label, slab,
};
use crate::ui::{eid, handler};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    if !app.notifications.open {
        return div().into_any_element();
    }
    let view = cx.entity();
    let state = &app.notifications;

    let mut body = div().flex().flex_col().gap_3().pt_3();

    if !state.wire.mutes.is_empty() {
        body = body.child(rules(app, cx));
    }
    if let Some(pick) = state.muting.as_ref() {
        body = body.child(picker(pick.scope.label(), pick.max_level, cx));
    }
    body = if state.wire.items.is_empty() {
        body.child(empty())
    } else {
        body.child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .children(state.wire.items.iter().map(|item| row(item, cx))),
        )
    };

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "notifications-mute-all",
            Some(IconName::EyeOff),
            "Mute everything",
            cx.listener(|this, _, _, cx| this.begin_mute(MuteScope::All, cx)),
        ))
        .child(ghost_button(
            "notifications-read-all",
            Some(IconName::Check),
            "Mark all read",
            cx.listener(|this, _, _, _| this.read_notification(None)),
        ))
        .child(ghost_button(
            "notifications-clear-all",
            Some(IconName::Delete),
            "Clear all",
            cx.listener(|this, _, _, _| this.dismiss_notification(None)),
        ))
        .into_any_element();

    modal(
        "notifications",
        theme::accent(),
        "Notifications",
        body.into_any_element(),
        footer,
        handler(&view, |this, _, cx| this.close_notifications(cx)),
        window,
    )
}

/// The colour a level reads in. There is no error token distinct from `danger`, and no need for
/// one: an error and something irreversible are the same red in this window on purpose.
fn colour_of(level: Level) -> Rgba {
    match level {
        Level::Info => theme::info(),
        Level::Warning => theme::warning(),
        Level::Error => theme::danger(),
    }
}

/// The rules in force, each with the control that lifts it.
fn rules(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(section_label("Silenced"))
        .children(
            app.notifications
                .wire
                .mutes
                .iter()
                .enumerate()
                .map(|(index, rule)| rule_row(index, rule, cx)),
        )
        .into_any_element()
}

fn rule_row(index: usize, rule: &MuteRule, cx: &mut Context<AppState>) -> AnyElement {
    let scope = rule.scope.clone();
    slab(theme::text_faint())
        .flex_row()
        .items_center()
        .gap_2()
        .py_1p5()
        .pr_2()
        .pl_2()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text())
                        .child(rule.scope.label()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::text_faint())
                        .child(format!(
                            "at or below {} · {}",
                            rule.max_level.label(),
                            lapses(rule.until)
                        )),
                ),
        )
        .child(
            icon_button(
                ("notification-unmute", index),
                IconName::Eye,
                false,
                cx.listener(move |this, _, _, _| this.unmute(scope.clone())),
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Turn this back on").build(window, cx)
            }),
        )
        .into_any_element()
}

/// When a rule lapses, as a person reads it. A rule with no end says so rather than printing a
/// date nobody set.
fn lapses(until: Option<u64>) -> String {
    let Some(until) = until else {
        return "until turned back on".to_string();
    };
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    if until <= now {
        return "lapsed".to_string();
    }
    let minutes = (until - now) / 60_000;
    match minutes {
        0 => "for under a minute".to_string(),
        1..=59 => format!("for {minutes}m"),
        _ => format!("for {}h", minutes / 60),
    }
}

/// One notification. A row with a link is what is clicked to get there; one without is text.
fn row(item: &Notification, cx: &mut Context<AppState>) -> AnyElement {
    let colour = colour_of(item.level);
    let id = item.id;
    let body = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .child(chip(item.family.label(), colour))
                .children(
                    item.actor
                        .as_ref()
                        .map(|actor| chip(actor, theme::border())),
                )
                .children(
                    item.category
                        .as_ref()
                        .map(|category| chip(category, theme::border())),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    mono(when_text(item.at), theme::text_faint())
                        .flex_none()
                        .text_size(px(10.5)),
                ),
        )
        .child(
            div()
                .w(relative(1.))
                .text_size(px(12.5))
                .text_color(if item.read {
                    theme::text_muted()
                } else {
                    theme::text()
                })
                .child(item.text.clone()),
        );

    let controls = div()
        .flex()
        .flex_none()
        .items_center()
        .child(marks(item))
        .children((!item.read).then(|| {
            icon_button(
                eid("notification-read", id),
                IconName::Check,
                false,
                cx.listener(move |this, _, _, _| this.read_notification(Some(id))),
            )
        }))
        .child(mute_control(item, cx))
        .child(icon_button(
            eid("notification-drop", id),
            IconName::Delete,
            false,
            cx.listener(move |this, _, _, _| this.dismiss_notification(Some(id))),
        ));

    // A row that goes somewhere is a card: it takes the click, and reading it is part of
    // following it — a notification you have acted on is not still waiting for you.
    match item.link.clone() {
        Some(link) => card(eid("notification", id), colour, false)
            .flex_row()
            .items_start()
            .gap_2()
            .p_2()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.read_notification(Some(id));
                this.follow_notification_link(link.clone(), window, cx);
            }))
            .child(body)
            .child(controls)
            .into_any_element(),
        None => slab(colour)
            .flex_row()
            .items_start()
            .gap_2()
            .p_2()
            .child(body)
            .child(controls)
            .into_any_element(),
    }
}

/// The two marks a row can wear: unread, and arrived quietly.
fn marks(item: &Notification) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .pr_1()
        .children(item.muted.then(|| {
            Icon::new(IconName::EyeOff)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint())
        }))
        .children((!item.read).then(|| div().size(px(6.)).rounded_full().bg(theme::accent())))
        .into_any_element()
}

/// The mute control on a row, offering the **narrowest** scope the row supports: its category,
/// then its actor, then its family. A row is silenced by what it actually is, so muting the one
/// noisy thing never takes the rest of its family with it.
fn mute_control(item: &Notification, cx: &mut Context<AppState>) -> AnyElement {
    let (scope, label) = narrowest(item);
    let tip: gpui::SharedString = format!("Silence {label}").into();
    icon_button(
        eid("notification-mute", item.id),
        IconName::EyeOff,
        false,
        cx.listener(move |this, _, _, cx| this.begin_mute(scope.clone(), cx)),
    )
    .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
    .into_any_element()
}

fn narrowest(item: &Notification) -> (MuteScope, String) {
    if let Some(category) = item.category.clone() {
        let scope = MuteScope::Category {
            family: item.family,
            category,
        };
        let label = scope.label();
        return (scope, label);
    }
    if let Some(actor) = item.actor.clone() {
        let scope = MuteScope::Actor {
            family: item.family,
            actor,
        };
        let label = scope.label();
        return (scope, label);
    }
    (
        MuteScope::Family(item.family),
        item.family.label().to_string(),
    )
}

/// The mute picker: what is being silenced, how loud it may still be, and for how long. The
/// duration is the confirm — picking one sends the rule — so there is no second button.
fn picker(scope: String, level: Level, cx: &mut Context<AppState>) -> AnyElement {
    let levels: Vec<AnyElement> = Level::ALL
        .iter()
        .map(|candidate| {
            let candidate = *candidate;
            choice_pill(
                ("notification-mute-level", candidate as usize),
                candidate.label(),
                candidate == level,
                cx.listener(move |this, _, _, cx| this.set_mute_level(candidate, cx)),
            )
            .into_any_element()
        })
        .collect();

    let durations: Vec<AnyElement> = MuteFor::ALL
        .iter()
        .map(|duration| {
            let duration = *duration;
            ghost_button(
                ("notification-mute-for", duration as usize),
                None,
                duration.label(),
                cx.listener(move |this, _, _, cx| this.confirm_mute(duration, cx)),
            )
            .into_any_element()
        })
        .collect();

    slab(theme::warning())
        .gap_2()
        .p_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(section_label("Silence"))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.5))
                        .text_color(theme::text())
                        .child(scope),
                )
                .child(ghost_button(
                    "notification-mute-cancel",
                    None,
                    "Cancel",
                    cx.listener(|this, _, _, cx| this.cancel_mute(cx)),
                )),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::text_faint())
                .child("silence at or below…"),
        )
        .child(div().flex().flex_wrap().gap_1p5().children(levels))
        .child(div().flex().flex_wrap().gap_1().children(durations))
        .into_any_element()
}

/// The page a bell with nothing behind it draws.
fn empty() -> AnyElement {
    div()
        .py_6()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .child(
            Icon::new(IconName::Bell)
                .with_size(Size::Medium)
                .text_color(theme::text_faint()),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme::text_faint())
                .child("Nothing has come up."),
        )
        .into_any_element()
}

/// A small tag for one part of a row's origin.
fn chip(label: impl Into<gpui::SharedString>, edge: Rgba) -> AnyElement {
    pill(edge)
        .h(px(18.))
        .px_1p5()
        .gap_1()
        .child(mono(label, theme::text_muted()).text_size(px(10.5)))
        .into_any_element()
}

/// How long ago it arrived, in the same short form the rest of the window uses. The stamp is
/// epoch milliseconds on the wire; a clock that cannot be read reads as the present.
fn when_text(at: u64) -> String {
    match chrono::DateTime::from_timestamp_millis(at as i64) {
        Some(then) => when::relative(then, chrono::Utc::now()),
        None => when::NEVER.to_string(),
    }
}
