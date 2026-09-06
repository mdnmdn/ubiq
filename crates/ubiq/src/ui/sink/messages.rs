//! The messages page: one live conversation, beside the bus traffic behind it.
//!
//! Every other sink page draws a fixture. This one draws the real thing twice over — what the chat
//! surface makes of a conversation on the left, and what actually travelled to produce it on the
//! right — because the gap between those two is where a conversation bug lives, and reading either
//! half alone never finds it.
//!
//! **The sink has no project**, so the conversation is picked out of whatever the window holds
//! rather than looked up in one: every open project's conversations, named in a row of pills. The
//! tape on the right is process-wide and belongs to [`ubiq_proto::bus`]; this reads it and never
//! writes to it, clearing the ring being the one thing it asks — exactly the log console's bargain.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::agents::SINK_SLOT;
use crate::theme;
use crate::ui::conversation::ConversationView;
use crate::ui::empty::empty_page;
use crate::ui::kit::{choice_pill, ghost_button, icon_button, mono};
use ubiq_proto::bus::{Direction, TapeEntry, tape};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .border_r_1()
                .border_color(theme::border())
                .child(chat(app, window, cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(viewer(app, cx)),
        )
        .into_any_element()
}

// ── The left half ───────────────────────────────────────────────────

/// The conversations the window holds, the one being read, and the control that starts another.
///
/// *New chat* is the agents screen's own New agent menu — the shell paints it, so this only asks
/// for it — and the conversation it starts is the one this page then reads.
fn chat(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let choices = app.sink_conversations();
    let selected = app.sink_agent();

    let mut pills = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .px_2()
        .py_1p5()
        .flex_none()
        .border_b_1()
        .border_color(theme::border());
    for (id, name) in &choices {
        let agent = *id;
        pills = pills.child(choice_pill(
            gpui::ElementId::Name(format!("sink-conversation-{agent}").into()),
            name.clone(),
            Some(agent) == selected,
            cx.listener(move |this, _, _, cx| this.set_sink_conversation(agent, cx)),
        ));
    }
    pills = pills.child(div().flex_1()).child(ghost_button(
        "sink-new-chat",
        Some(IconName::Plus),
        "New chat",
        cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
            let at = event.position();
            this.start_sink_chat((at.x.into(), at.y.into()), cx);
        }),
    ));

    let surface = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(pills);

    match selected.and_then(|agent| app.sink_conversation(agent)) {
        Some(conversation) => surface
            .child(crate::ui::conversation::render(
                app,
                conversation,
                ConversationView {
                    id: "sink-messages".into(),
                    slot: SINK_SLOT,
                    footer: true,
                    composer: true,
                    header: true,
                },
                window,
                cx,
            ))
            .into_any_element(),
        // Nothing to read yet, and the control that fixes it is right above this line rather than
        // named in prose the reader has to go looking for.
        None => surface
            .child(empty_page(
                "No conversations",
                "The sink has no project of its own. Start one with New chat, or start an agent \
                 in a project and it shows up here with the bus traffic behind it.",
                IconName::Asterisk,
                None,
            ))
            .into_any_element(),
    }
}

// ── The right half ──────────────────────────────────────────────────

/// Which way an entry went, and so what colour it is drawn in: inbound is from elsewhere, outbound
/// is from us. Direction is never shown by the arrow alone.
fn direction_colour(direction: Direction) -> Rgba {
    match direction {
        Direction::Inbound => theme::info(),
        Direction::Outbound => theme::accent(),
    }
}

fn arrow(direction: Direction) -> &'static str {
    match direction {
        Direction::Inbound => "\u{2190}",
        Direction::Outbound => "\u{2192}",
    }
}

/// The tape: its toolbar, the list of what crossed, and the one message being read under it.
///
/// A row says only when, which way, which message and how big it is — the message itself is far
/// too tall for a row, so it is drawn once, formatted, in the half below rather than expanded in
/// place.
fn viewer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let entries = tape().snapshot();
    let state = &app.sink.messages;

    // Following means the tail stays in view as entries arrive. The handle defers the request, so
    // it costs nothing on a page nobody is looking at.
    if state.follow && !entries.is_empty() {
        state.scroll.scroll_to_item(entries.len() - 1);
    }

    let mut list = div()
        .id("sink-tape")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .track_scroll(&state.scroll);

    // ponytail: every entry is laid out, not just the visible ones. The ring is capped at
    // TAPE_CAPACITY and this is a bench page; make it a uniform_list if that stops being true.
    for entry in &entries {
        list = list.child(row(app, entry, cx));
    }

    let body: AnyElement = if entries.is_empty() {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .child(mono("Nothing on the bus yet.", theme::text_faint()))
            .into_any_element()
    } else {
        list.into_any_element()
    };

    let selected = state
        .selected
        .and_then(|seq| entries.iter().find(|entry| entry.seq == seq).cloned());

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .bg(theme::pane_bg())
        .child(actions(app, cx))
        .children(state.dumped.clone().map(|path| {
            div()
                .px_2()
                .py_1()
                .flex()
                .flex_none()
                .border_b_1()
                .border_color(theme::border())
                .child(mono(path, theme::text_muted()).text_size(px(11.)))
        }))
        .child(body)
        .child(formatted(app, selected.as_ref()))
        .into_any_element()
}

/// What the tape is showing, and the four controls that decide.
fn actions(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let state = &app.sink.messages;
    let (kept, dropped) = tape().counts();
    let counted = if dropped == 0 {
        format!("{kept} messages")
    } else {
        format!("{kept} messages \u{b7} {dropped} dropped")
    };

    div()
        .h(px(34.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .border_b_1()
        .border_color(theme::border())
        .child(choice_pill(
            "sink-tape-json",
            "Message",
            !state.original,
            cx.listener(|this, _, _, cx| this.set_sink_message_original(false, cx)),
        ))
        .child(choice_pill(
            "sink-tape-raw",
            "Original",
            state.original,
            cx.listener(|this, _, _, cx| this.set_sink_message_original(true, cx)),
        ))
        .child(div().flex_1())
        .child(mono(counted, theme::text_faint()).text_size(px(11.)))
        .child(icon_button(
            "sink-tape-follow",
            IconName::ArrowDown,
            state.follow,
            cx.listener(|this, _, _, cx| this.toggle_sink_message_follow(cx)),
        ))
        .child(ghost_button(
            "sink-tape-dump",
            None,
            "Dump",
            cx.listener(|this, _, _, cx| this.dump_sink_messages(cx)),
        ))
        .child(ghost_button(
            "sink-tape-clear",
            Some(IconName::Delete),
            "Clear",
            cx.listener(|this, _, _, cx| this.clear_sink_messages(cx)),
        ))
        .into_any_element()
}

/// The body being read, for whichever of the two the toggle is on. `None` is an entry with no
/// original — a message this half of the bus minted itself.
fn body_of(entry: &TapeEntry, original: bool) -> Option<&str> {
    if original {
        entry.raw.as_deref()
    } else {
        Some(entry.json.as_str())
    }
}

/// How big a message is, in the units a reader reads sizes in.
fn size_of(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f32 / 1024.0)
    }
}

/// One entry: when, which way, which message, and how big it is. Nothing of the message itself —
/// that is the viewer's half, under the list.
fn row(app: &AppState, entry: &Arc<TapeEntry>, cx: &mut Context<AppState>) -> AnyElement {
    let colour = direction_colour(entry.direction);
    let seq = entry.seq;
    let selected = app.sink.messages.selected == Some(seq);

    div()
        .id(gpui::ElementId::Name(format!("sink-tape-{seq}").into()))
        .h(px(22.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .overflow_hidden()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .when(selected, |this| {
            this.bg(theme::accent_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(colour)
        })
        .on_click(cx.listener(move |this, _, _, cx| this.select_sink_message(seq, cx)))
        .child(
            mono(time(entry), theme::text_faint())
                .flex_none()
                .text_size(px(11.)),
        )
        .child(mono(arrow(entry.direction), colour).flex_none().w(px(14.)))
        .child(
            mono(entry.kind.clone(), colour)
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.))
                .overflow_hidden(),
        )
        .child(
            mono(size_of(entry.json.len()), theme::text_muted())
                .flex_none()
                .text_size(px(11.)),
        )
        .into_any_element()
}

/// The message the list picked, formatted — the half of the page a bug is actually read in.
///
/// JSON is re-indented rather than printed as it travelled: a one-line frame is unreadable, and
/// the tape truncates a long one, so anything that will not parse is shown exactly as it came.
fn formatted(app: &AppState, entry: Option<&Arc<TapeEntry>>) -> AnyElement {
    let state = &app.sink.messages;

    let mut pane = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .border_t_1()
        .border_color(theme::border())
        .bg(theme::app_bg());

    let Some(entry) = entry else {
        return pane
            .items_center()
            .justify_center()
            .child(mono("Pick a message to read it.", theme::text_faint()).text_size(px(11.5)))
            .into_any_element();
    };

    let colour = direction_colour(entry.direction);
    let text = match body_of(entry, state.original) {
        Some(body) => pretty(body),
        None => "This message carried no original line.".to_string(),
    };

    pane = pane.child(
        div()
            .h(px(24.))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(mono(arrow(entry.direction), colour).text_size(px(11.)))
            .child(mono(entry.kind.clone(), colour).text_size(px(11.)))
            .child(div().flex_1())
            .child(
                mono(
                    if state.original {
                        "original"
                    } else {
                        "message"
                    },
                    theme::text_faint(),
                )
                .text_size(px(10.5)),
            ),
    );

    pane.child(
        div()
            .id("sink-tape-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .px_2()
            .pb_2()
            .overflow_scroll()
            .children(
                text.lines()
                    .map(|line| mono(line.to_string(), theme::text()).text_size(px(11.)))
                    .collect::<Vec<_>>(),
            ),
    )
    .into_any_element()
}

/// Re-indented JSON, or the text unchanged when it is not JSON — a truncated frame is both, and
/// showing it raw beats showing a parse error for something the reader can already read.
fn pretty(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| body.to_string())
}

/// When it went, to the millisecond, the way the log console prints one.
fn time(entry: &TapeEntry) -> String {
    chrono::DateTime::<chrono::Local>::from(entry.at)
        .format("%H:%M:%S%.3f")
        .to_string()
}
