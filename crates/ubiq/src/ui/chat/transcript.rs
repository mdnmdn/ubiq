//! The conversation itself: user turns, assistant turns, and the tool blocks inside them.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, Rgba,
    SharedString, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::scroll::Scrollbar;
use gpui_component::text::TextView;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::{Block, ChatMessage, DiffKind, ToolCall, ToolKind};
use crate::theme;
use crate::ui::kit::mono;

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let mut turns: Vec<AnyElement> = Vec::new();
    if let Some(chat) = app.chat.active_chat() {
        for (ix, message) in chat.messages.iter().enumerate() {
            turns.push(match message {
                ChatMessage::User(text) => user_turn(text).into_any_element(),
                ChatMessage::Assistant(blocks) => assistant_turn(ix, blocks, cx),
            });
        }
    }

    // The scrollbar is a sibling of the scroll area, so it stays put while the content moves.
    div()
        .relative()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .id("chat-transcript")
                .size_full()
                .p_3()
                .flex()
                .flex_col()
                .gap_4()
                .overflow_y_scroll()
                .track_scroll(&app.chat_scroll)
                .children(if turns.is_empty() {
                    vec![
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(mono("nothing said yet", theme::text_faint()))
                            .into_any_element(),
                    ]
                } else {
                    turns
                }),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .child(Scrollbar::vertical(&app.chat_scroll)),
        )
}

fn user_turn(text: &str) -> impl IntoElement {
    div()
        .pl_3()
        .py_2()
        .pr_3()
        .flex()
        .flex_none()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .text_size(px(13.5))
        .text_color(theme::text())
        .child(SharedString::from(text.to_string()))
}

fn assistant_turn(message: usize, blocks: &[Block], cx: &mut Context<AppState>) -> AnyElement {
    let mut rendered: Vec<AnyElement> = Vec::new();
    for (ix, block) in blocks.iter().enumerate() {
        rendered.push(match block {
            Block::Markdown(source) => TextView::markdown(
                ElementId::Name(format!("md-{message}-{ix}").into()),
                source.clone(),
            )
            .into_any_element(),
            Block::Tool(tool) => tool_block(message, ix, tool, cx),
        });
    }

    div()
        .flex()
        .flex_none()
        .gap_3()
        .child(
            div()
                .size(px(26.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .bg(theme::surface())
                .child(
                    Icon::new(IconName::Asterisk)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_muted()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap_2()
                .text_size(px(13.5))
                .children(rendered),
        )
        .into_any_element()
}

/// The colour a tool block is filed under. Read and grep are informational; an edit is a change;
/// a command is something that ran.
fn tool_colour(kind: ToolKind) -> Rgba {
    match kind {
        ToolKind::Read | ToolKind::Grep => theme::accent(),
        ToolKind::Edit => theme::warning(),
        ToolKind::Bash => theme::success(),
    }
}

fn tool_block(
    message: usize,
    block: usize,
    tool: &ToolCall,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = tool_colour(tool.kind);
    let expandable = tool.has_body();

    let header = div()
        .id(ElementId::Name(format!("tool-{message}-{block}").into()))
        .h(px(30.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(if tool.expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::XSmall)
            .text_color(theme::text_faint()),
        )
        .child(mono(tool.kind.label(), colour).text_size(px(11.5)))
        .child(
            mono(tool.target.clone(), theme::text())
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.)),
        )
        .child(mono(tool.meta.clone(), theme::text_muted()).text_size(px(11.5)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_tool(message, block, cx);
        }));

    let mut card = div()
        .flex()
        .flex_col()
        .flex_none()
        .bg(theme::pane_bg())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(colour)
        .child(header);

    if tool.expanded && expandable {
        let body = if tool.diff.is_empty() {
            div()
                .flex()
                .flex_col()
                .children(
                    tool.body
                        .iter()
                        .map(|line| mono(line.clone(), theme::text_muted()).text_size(px(11.5)))
                        .collect::<Vec<_>>(),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .children(tool.diff.iter().map(diff_line).collect::<Vec<_>>())
                .into_any_element()
        };

        card = card.child(
            div()
                .px_2()
                .py_2()
                .border_t_1()
                .border_color(theme::border())
                .child(body),
        );
    }

    card.into_any_element()
}

fn diff_line(line: &crate::state::DiffLine) -> impl IntoElement {
    let (fg, bg) = match line.kind {
        DiffKind::Add => (theme::success(), theme::success_soft()),
        DiffKind::Remove => (theme::danger(), theme::danger_soft()),
        DiffKind::Context => (theme::text_muted(), theme::pane_bg()),
    };

    div()
        .flex()
        .flex_none()
        .gap_2()
        .px_1()
        .bg(bg)
        .child(mono(line.kind.marker(), fg).text_size(px(11.5)))
        .child(
            mono(line.text.clone(), fg)
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.5)),
        )
}
