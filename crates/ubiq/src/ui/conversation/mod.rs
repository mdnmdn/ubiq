//! One live agent's conversation, drawn once for every surface that shows one.
//!
//! There is a single interface for talking to an agent, and this is it. The agents screen's
//! columns host it today; the chat panel and the style reference are the next two, and neither
//! needs a second renderer to do it — what differs between hosts is [`ConversationView`], which
//! says whether the footer and the composer come with it and which of the window's pooled
//! composers to type into.
//!
//! **Nothing here knows which screen it is inside.** It is handed a [`Conversation`] and draws it:
//! no column, no tab, no slot of the agents screen's own arrangement reaches in. That is the whole
//! constraint, and it is what lets a second host adopt this by passing a different view.
//!
//! Nothing here appends to a transcript either. The composer sends and the line appears when the
//! harness echoes it back — an interface that draws its own half of a conversation is inventing
//! the other half too.

use gpui::{
    AnyElement, Context, ElementId, Focusable, InteractiveElement, IntoElement, ParentElement,
    Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::Textarea;
use gpui_component::text::TextView;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::conversation::{ConfigCategory, ConfigValue, ToolContent, ToolKind, ToolStatus};
use ubiq_proto::work::AgentId;

use crate::app::AppState;
use crate::state::conversation::{ConvBlock, Conversation, Pending, QueuedMessage, Run};
use crate::theme;
use crate::ui::kit::menu::MENU_ANCHOR_UP;
use crate::ui::kit::{
    HARNESS_GLYPH, Picker, PickerStyle, field, ghost_button, mono, pill, progress_ring, state_chip,
};
use crate::ui::{handler, indexed};

/// What differs between the surfaces that host a conversation.
pub struct ConversationView {
    /// What every element id inside is built from, so two conversations on screen at once do not
    /// collide. A prefix rather than an [`ElementId`], because the ids under it are composed.
    pub id: SharedString,
    /// Which of the window's pooled composer fields this surface types into. An index, and
    /// nothing else: which surface owns which slot is the host's question.
    pub slot: usize,
    pub footer: bool,
    pub composer: bool,
}

impl ConversationView {
    fn eid(&self, part: &str) -> ElementId {
        ElementId::Name(format!("{}-{part}", self.id).into())
    }
}

pub fn render(
    app: &AppState,
    conversation: &Conversation,
    view: ConversationView,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;

    let mut root = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(transcript(conversation, &view, cx));

    if let Some(pending) = &conversation.pending {
        root = root.child(permission(id, pending, &view, cx));
    }
    if let Some(error) = &conversation.error {
        root = root.child(
            div()
                .px_3()
                .py_1p5()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .bg(theme::danger_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::danger())
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .with_size(Size::XSmall)
                        .text_color(theme::danger()),
                )
                .child(mono(error.clone(), theme::text()).text_size(px(11.5))),
        );
    }
    if view.footer {
        root = root.child(footer(conversation));
    }
    if view.composer {
        root = root.child(composer(app, conversation, &view, window, cx));
    }

    root.into_any_element()
}

/// What has been said, oldest first.
fn transcript(
    conversation: &Conversation,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    let blocks: Vec<AnyElement> = conversation
        .blocks
        .iter()
        .enumerate()
        .map(|(ix, block)| match block {
            ConvBlock::User(text) => user_turn(text),
            ConvBlock::Agent(body) => TextView::markdown(
                view.eid(&format!("md-{ix}")),
                SharedString::from(body.clone()),
            )
            .into_any_element(),
            ConvBlock::Thought(body) => thought(body),
            ConvBlock::Tool { call, open } => tool_block(id, ix, call, *open, view, cx),
        })
        .collect();

    div()
        .id(view.eid("transcript"))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .py_2()
        .gap_2()
        .text_size(px(13.5))
        .text_color(theme::text())
        .overflow_y_scroll()
        .children(if blocks.is_empty() {
            vec![
                mono("nothing said yet", theme::text_faint())
                    .text_size(px(11.5))
                    .into_any_element(),
            ]
        } else {
            blocks
        })
        .into_any_element()
}

/// What the user said sits in the accent, the way every other surface in the window draws a turn
/// of theirs.
fn user_turn(text: &str) -> AnyElement {
    div()
        .pl_6()
        .flex_none()
        .child(
            div()
                .p_2()
                .bg(theme::accent_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent())
                .text_size(px(13.))
                .text_color(theme::text())
                .child(SharedString::from(text.to_string())),
        )
        .into_any_element()
}

/// Reasoning, quieter than prose: it is what the agent thought on the way to what it said, and it
/// must not read as the answer.
fn thought(body: &str) -> AnyElement {
    div()
        .p_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::text_faint())
        .child(mono("THINKING", theme::text_faint()).text_size(px(10.5)))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child(SharedString::from(body.to_string())),
        )
        .into_any_element()
}

/// The colour a tool block is filed under, on the four readings the chat panel already uses: a
/// look is informational, a change is a change, a removal is destructive, a command ran. The ten
/// ACP kinds share them rather than growing ten tokens nobody could tell apart.
fn tool_colour(kind: ToolKind) -> Rgba {
    match kind {
        ToolKind::Read | ToolKind::Search | ToolKind::Fetch => theme::accent(),
        ToolKind::Edit | ToolKind::Move => theme::warning(),
        ToolKind::Delete => theme::danger(),
        ToolKind::Execute => theme::success(),
        ToolKind::Think | ToolKind::SwitchMode => theme::info(),
        ToolKind::Other => theme::text_muted(),
    }
}

fn status_colour(status: ToolStatus) -> Rgba {
    match status {
        ToolStatus::Pending => theme::text_faint(),
        ToolStatus::InProgress => theme::accent(),
        ToolStatus::Completed => theme::success(),
        ToolStatus::Failed => theme::danger(),
    }
}

fn status_label(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "pending",
        ToolStatus::InProgress => "running",
        ToolStatus::Completed => "done",
        ToolStatus::Failed => "failed",
    }
}

/// A tool's title: plain for most kinds, a code chip for a command. A command reads the way one
/// does in the app's own markdown — monospace on a raised surface — because the title has no room
/// for the coloured edge every other surface here is identified by.
fn tool_title(kind: ToolKind, title: String) -> gpui::Div {
    let text = mono(title, theme::text()).text_size(px(12.));
    if kind != ToolKind::Execute {
        return text;
    }
    div()
        .bg(theme::surface_raised())
        .px_1()
        .py(px(1.))
        .child(text)
}

/// A tool call: what it did, to what, and how it went — before any of what it produced.
fn tool_block(
    agent: AgentId,
    index: usize,
    call: &ubiq_proto::conversation::ToolCallRecord,
    open: bool,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = tool_colour(call.kind);
    let expandable = !call.content.is_empty();
    let call_id = call.id.clone();

    // A title that wraps — a long command, most often — grows the row rather than being
    // clipped to one line's height: `min_h` is the floor, not the ceiling, and `items_start`
    // keeps the icon and the status pinned to the first line instead of drifting to the
    // paragraph's centre.
    let mut header = div()
        .id(view.eid(&format!("tool-{index}")))
        .min_h(px(30.))
        .px_2()
        .py_1()
        .flex()
        .flex_none()
        .items_start()
        .gap_2()
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::XSmall)
            .text_color(if expandable {
                theme::text_faint()
            } else {
                theme::border()
            })
            .mt(px(2.)),
        )
        .child(
            mono(call.kind.label(), colour)
                .text_size(px(11.5))
                .mt(px(1.)),
        )
        .child(
            tool_title(call.kind, call.title.clone())
                .flex_1()
                .min_w(px(0.)),
        )
        .child(
            mono(status_label(call.status), status_colour(call.status))
                .text_size(px(11.5))
                .mt(px(1.)),
        );

    // A block with nothing behind it does not expand: a chevron that opens on emptiness says the
    // detail is missing rather than absent.
    if expandable {
        header = header
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_conversation_tool(agent, call_id.clone(), cx);
            }));
    }

    let mut card = div()
        .flex()
        .flex_col()
        .flex_none()
        .bg(theme::pane_bg())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(colour)
        .child(header);

    if open && expandable {
        card = card.child(
            div()
                .px_2()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .border_t_1()
                .border_color(theme::border())
                .children(call.content.iter().map(content).collect::<Vec<_>>()),
        );
    }

    card.into_any_element()
}

fn content(item: &ToolContent) -> AnyElement {
    match item {
        ToolContent::Text(text) => div()
            .flex()
            .flex_col()
            .children(
                text.lines()
                    .map(|line| mono(line.to_string(), theme::text_muted()).text_size(px(11.5)))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        ToolContent::Diff {
            path,
            old_text,
            new_text,
        } => diff(path, old_text.as_deref(), new_text),
    }
}

/// An edit, as the block that went and the block that came. Line for line rather than matched:
/// there is no diff engine here, and a bad alignment reads as changes nobody made.
fn diff(path: &str, old_text: Option<&str>, new_text: &str) -> AnyElement {
    let mut rows: Vec<AnyElement> = vec![
        mono(path.to_string(), theme::text_muted())
            .text_size(px(11.))
            .into_any_element(),
    ];
    if let Some(old) = old_text {
        rows.extend(
            old.lines()
                .map(|line| diff_line("-", line, theme::danger(), theme::danger_soft())),
        );
    }
    rows.extend(
        new_text
            .lines()
            .map(|line| diff_line("+", line, theme::success(), theme::success_soft())),
    );

    div().flex().flex_col().children(rows).into_any_element()
}

fn diff_line(marker: &str, text: &str, fg: Rgba, bg: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .gap_2()
        .px_1()
        .bg(bg)
        .child(mono(marker.to_string(), fg).text_size(px(11.5)))
        .child(
            mono(text.to_string(), fg)
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.5)),
        )
        .into_any_element()
}

/// What the agent is asking to be allowed to do, and the answers it offered.
///
/// Nothing emits one of these today — every bridge auto-approves — but the vocabulary carries the
/// request, and a surface that could not draw it would have to grow one the day a bridge stops.
fn permission(
    agent: AgentId,
    pending: &Pending,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let what = pending
        .tool_call
        .title
        .clone()
        .unwrap_or_else(|| "The agent is asking to go ahead".to_string());

    let buttons: Vec<AnyElement> = pending
        .options
        .iter()
        .enumerate()
        .map(|(ix, option)| {
            let request_id = pending.request_id.clone();
            let option_id = option.option_id.clone();
            ghost_button(
                view.eid(&format!("permission-{ix}")),
                None,
                option.name.clone(),
                cx.listener(move |this, _, _, cx| {
                    this.answer_permission(agent, request_id.clone(), option_id.clone(), cx);
                }),
            )
            .into_any_element()
        })
        .collect();

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_2()
        .bg(theme::warning_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::warning())
        .child(mono("NEEDS YOU", theme::warning()).text_size(px(10.5)))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text())
                .child(SharedString::from(what)),
        )
        .child(div().flex().items_center().gap_2().children(buttons))
        .into_any_element()
}

/// What the harness said about itself: which one it is, which model, what the turn has cost, and
/// how much of the context window is gone.
///
/// The ring is drawn only where a window was reported. A percentage of a size nobody named is a
/// wrong ring, and a wrong ring is worse than none.
fn footer(conversation: &Conversation) -> AnyElement {
    let mut row = div()
        .px_3()
        .py_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .border_t_1()
        .border_color(theme::border())
        .child(
            pill(theme::accent())
                .h(px(22.))
                .px_2()
                .child(mono(HARNESS_GLYPH, theme::text()).text_size(px(11.))),
        );

    // Which identity answered. Read-only by design: it is chosen once, in the New agent menu,
    // because a turn already taken was taken as somebody. Drawn only when the run resolved one
    // — an empty pill beside the harness would claim an identity that does not exist.
    if !conversation.account.is_empty() {
        row = row.child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono(conversation.account.clone(), theme::text_muted()).text_size(px(11.))),
        );
    }
    if let Some(model) = &conversation.model {
        row = row.child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono(model.clone(), theme::text()).text_size(px(11.))),
        );
    }
    if let Some(mode) = &conversation.mode {
        row = row.child(state_chip(mode.clone(), theme::info(), 1.0));
    }

    row = row.child(div().flex_1().min_w(px(0.)));

    if let Some(cost) = conversation.cost_usd() {
        row = row.child(mono(format!("${cost:.2}"), theme::text_muted()).text_size(px(11.)));
    }
    if let Some(pct) = conversation.context_pct() {
        row = row.child(progress_ring(pct, 12.)).child(
            mono(
                format!("{:.1}K ctx", conversation.tokens() as f32 / 1000.0),
                theme::text_muted(),
            )
            .text_size(px(11.)),
        );
    }
    if let Some(pct) = conversation.rate_limit_five_hour_pct() {
        row = row.child(mono(format!("5h {pct}%"), theme::text_muted()).text_size(px(11.)));
    }

    row.into_any_element()
}

/// The field that steers this agent, and the Stop that interrupts it.
fn composer(
    app: &AppState,
    conversation: &Conversation,
    view: &ConversationView,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    // A harness that takes no second turn says so where the field would be, rather than leaving
    // one that swallows what is typed.
    if !conversation.accepts_input {
        return div()
            .px_3()
            .py_2()
            .flex()
            .flex_none()
            .border_t_1()
            .border_color(theme::border())
            .child(
                mono(
                    "This agent has ended \u{2014} its transcript stays, and it takes no more turns.",
                    theme::text_faint(),
                )
                .text_size(px(11.5)),
            )
            .into_any_element();
    }

    let Some(input) = app.column_inputs.get(view.slot).cloned() else {
        return div().into_any_element();
    };
    let entity = cx.entity();
    let id = conversation.id;
    let slot = view.slot;
    let can_send = !input.read(cx).value().trim().is_empty();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let working = conversation.run == Run::Working;

    // Before the harness has launched, the composer offers a model picker instead of the
    // read-only pill the footer draws once it has — the picker's own row, above the field, rather
    // than blocking it: the user may type and send before discovery finishes, and the host then
    // launches with the harness's own default.
    let model_row = (!conversation.launched).then(|| {
        let option = conversation
            .config
            .iter()
            .find(|opt| opt.category == Some(ConfigCategory::Model));
        match option {
            None => div()
                .px_3()
                .py_1()
                .flex()
                .flex_none()
                .child(mono("Discovering models\u{2026}", theme::text_faint()).text_size(px(11.5)))
                .into_any_element(),
            Some(option) => {
                let current = match &option.value {
                    ConfigValue::Select { current, .. } => current.as_str(),
                    ConfigValue::Flag { .. } => "",
                };
                let chosen = conversation.chosen_model.as_deref().unwrap_or(current);
                let choices = match &option.value {
                    ConfigValue::Select { choices, .. } => choices.clone(),
                    ConfigValue::Flag { .. } => Vec::new(),
                };
                let selected = choices
                    .iter()
                    .position(|choice| choice.value == chosen)
                    .unwrap_or(0);
                let label = choices
                    .get(selected)
                    .map(|choice| choice.name.clone())
                    .unwrap_or_else(|| chosen.to_string());
                let values: Vec<String> =
                    choices.iter().map(|choice| choice.value.clone()).collect();
                let names: Vec<String> = choices.iter().map(|choice| choice.name.clone()).collect();

                div()
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(
                        Picker::new(view.eid("model-picker"), label)
                            .style(PickerStyle::Chip)
                            .anchor(MENU_ANCHOR_UP)
                            .items(names)
                            .selected(selected)
                            .open(conversation.model_menu_open)
                            .on_toggle(handler(&entity, move |this, _, cx| {
                                this.toggle_agent_model_menu(id, cx)
                            }))
                            .on_pick(indexed(&entity, move |this, index, _, cx| {
                                if let Some(value) = values.get(index) {
                                    this.pick_agent_model(id, value.clone(), cx);
                                }
                            }))
                            .on_dismiss(handler(&entity, move |this, _, cx| {
                                this.dismiss_agent_model_menu(id, cx)
                            })),
                    )
                    .into_any_element()
            }
        }
    });

    let controls = div()
        .px_2()
        .pb_2()
        .pt_1()
        .flex()
        .items_center()
        .gap_2()
        .child(
            mono(
                "\u{23ce} / cmd\u{2044}ctrl+\u{23ce} send \u{b7} \u{21e7}\u{23ce} newline",
                theme::text_faint(),
            )
            .text_size(px(10.5)),
        )
        .child(div().flex_1().min_w(px(0.)));

    // One control, and which it is depends on the turn and the draft: idle sends, a running turn
    // with nothing typed offers Stop, and a running turn with something typed queues it instead
    // of writing into a harness mid-turn — the same three states the Enter key answers through
    // `AppState::send_or_enqueue`, so the button and the key never disagree.
    let action = if working && !can_send {
        ghost_button(
            view.eid("stop"),
            Some(IconName::Close),
            "Stop",
            cx.listener(move |this, _, _, cx| this.cancel_turn(id, cx)),
        )
        .text_color(theme::danger())
    } else if working {
        ghost_button(
            view.eid("send"),
            Some(IconName::Inbox),
            "Enqueue",
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        )
        .text_color(theme::accent())
    } else {
        ghost_button(
            view.eid("send"),
            Some(IconName::ArrowUp),
            "Send",
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        )
        .text_color(if can_send {
            theme::accent()
        } else {
            theme::text_faint()
        })
    };

    let field_el = field(theme::accent(), focused)
        .flex_none()
        .flex_col()
        .items_stretch()
        .child(
            div()
                .id(view.eid("composer"))
                .px_3()
                .pt_2()
                .cursor_text()
                .child(
                    Textarea::new(&input)
                        .appearance(false)
                        .bordered(false)
                        .w_full()
                        .text_size(px(13.)),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let input = this.column_inputs[slot].clone();
                    input.update(cx, |state, cx| state.focus(window, cx));
                })),
        )
        .child(controls.child(action));

    let mut extras: Vec<AnyElement> = Vec::new();
    extras.extend(model_row);
    if !conversation.queued.is_empty() {
        extras.push(queue_list(id, slot, view, &conversation.queued, cx));
    }

    if extras.is_empty() {
        field_el.into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .children(extras)
            .child(field_el)
            .into_any_element()
    }
}

/// Prompts typed while a turn was running, oldest first — each with an edit that loads it back
/// into the field and a delete that drops it outright. Drawn only when there is one: an empty
/// queue draws nothing, the same discipline every pill in this file follows.
fn queue_list(
    agent_id: AgentId,
    slot: usize,
    view: &ConversationView,
    queued: &[QueuedMessage],
    cx: &mut Context<AppState>,
) -> AnyElement {
    const PREVIEW_CHARS: usize = 80;

    let rows: Vec<AnyElement> = queued
        .iter()
        .map(|message| {
            let queued_id = message.id;
            let mut preview: String = message.text.chars().take(PREVIEW_CHARS).collect();
            if message.text.chars().count() > PREVIEW_CHARS {
                preview.push('\u{2026}');
            }

            div()
                .id(view.eid(&format!("queued-{queued_id}")))
                .px_2()
                .h(px(26.))
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .bg(theme::surface())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::border())
                .child(
                    mono(preview, theme::text_muted())
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(11.5)),
                )
                .child(ghost_button(
                    view.eid(&format!("queued-edit-{queued_id}")),
                    Some(IconName::Replace),
                    "Edit",
                    cx.listener(move |this, _, window, cx| {
                        this.edit_queued_message(agent_id, slot, queued_id, window, cx);
                    }),
                ))
                .child(ghost_button(
                    view.eid(&format!("queued-delete-{queued_id}")),
                    Some(IconName::Delete),
                    "Delete",
                    cx.listener(move |this, _, _, cx| {
                        this.delete_queued_message(agent_id, queued_id, cx);
                    }),
                ))
                .into_any_element()
        })
        .collect();

    div()
        .px_2()
        .pt_1()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1()
        .children(rows)
        .into_any_element()
}
