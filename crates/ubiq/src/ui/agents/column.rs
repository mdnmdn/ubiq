//! One column of the agents screen: the tabs it holds, the agent in front, its thread, and the
//! field that steers it.
//!
//! A column is the whole of one conversation in one place, which is why it wears its own chrome
//! rather than borrowing the chat panel's: the tabs are agents, the header says what this one *is*
//! rather than which chat is open, and the footer reports what the host said about the harness
//! behind it. The left edge takes the active agent's activity colour, so a row of columns reads as
//! a row of states from across the window.
//!
//! **A tab is dragged, not reordered.** Dropped on another column it groups; dropped past the last
//! one it splits off. Both are the same gesture from the user's side, and neither sends anything —
//! the arrangement is this window's own.
//!
//! Nothing here writes into a transcript. What is typed reaches the host and the line appears in
//! the thread when the host answers with the agent carrying it: an interface that draws its own
//! half of a conversation is inventing the other half too.
//!
//! **A column draws one of two things below its header.** An agent the host is streaming is drawn
//! by [`crate::ui::conversation`], the one view every surface that shows a conversation shares; an
//! agent that is a record and nothing more keeps the thread and the composer below. The chrome
//! above is the same either way, because a column is a column whichever it holds.

use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use gpui_component::input::Textarea;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::{AgentId, Speaker, WorkAgent};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::agents::COLUMN_MIN_WIDTH;
use crate::state::work;
use crate::theme;
use crate::ui::agents::DraggedTab;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::kit::{
    HARNESS_GLYPH, Picker, PickerStyle, field, ghost_button, mono, pill, progress_ring,
    section_label, state_chip, status_dot,
};
use crate::ui::work::{activity_colour, role_mark};
use crate::ui::{eid, handler, indexed};

pub fn render(
    app: &AppState,
    column: usize,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (Some(work), Some(agents)) = (app.work(cx), app.agents(cx)) else {
        return div().into_any_element();
    };
    let Some(held) = agents.columns.get(column) else {
        return div().into_any_element();
    };
    let slot = held.slot;
    // A column always has an active tab: `prune` keeps the index inside the strip, and a column
    // with no tabs is removed rather than drawn empty.
    let Some(agent) = held.active_agent().and_then(|id| work.agent(id)) else {
        return div().into_any_element();
    };
    let colour = activity_colour(agent.activity);
    // The column a drop would group into says so by lighting up, which is the only answer the
    // user gets before letting go.
    let lit = agents
        .dragging
        .is_some_and(|dragged| !held.tabs.contains(&dragged));

    let tabs: Vec<AnyElement> = held
        .tabs
        .iter()
        .enumerate()
        .map(|(ix, id)| tab(app, column, ix, *id, ix == held.active, cx))
        .collect();

    let mut root = div()
        .id(("agents-column", column))
        .flex()
        .flex_1()
        .flex_col()
        .min_w(px(COLUMN_MIN_WIDTH))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(if lit { theme::accent() } else { colour })
        .on_drop(cx.listener(move |this, _: &DraggedTab, _, cx| this.drop_tab_on(column, cx)));

    if lit {
        root = root.bg(theme::accent_soft());
    }

    let strip = div()
        .h(px(38.))
        .flex()
        .flex_none()
        .items_center()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .children(tabs)
        .child(div().flex_1().min_w(px(0.)))
        .child(add_tab(app, column, cx));

    let root = root
        .child(strip)
        .child(header(agent, held.tabs.len(), work, colour));

    // A live agent is drawn by the one conversation view every surface shares; a mock keeps the
    // thread and the composer it has always had. Both are on screen at once, and which it is comes
    // down to whether the host is streaming this agent.
    match app.conversation(agent.id, cx) {
        Some(live) => root
            .child(conversation::render(
                app,
                live,
                ConversationView {
                    id: SharedString::from(format!("agents-column-{column}")),
                    slot,
                    footer: true,
                    composer: true,
                },
                window,
                cx,
            ))
            .into_any_element(),
        None => root
            .child(thread(app, agent.id, cx))
            .child(footer(agent))
            .child(composer(app, column, slot, window, cx))
            .into_any_element(),
    }
}

/// One tab: a state dot, the agent's name, and the close that benches it.
fn tab(
    app: &AppState,
    column: usize,
    index: usize,
    id: AgentId,
    active: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let Some(agent) = app.work(cx).and_then(|work| work.agent(id)) else {
        return div().into_any_element();
    };
    let name: SharedString = agent.name.clone().into();
    let ghost = name.clone();
    let colour = activity_colour(agent.activity);

    let mut row = div()
        .id(eid("agents-tab", id))
        .h(px(38.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_b_2()
        .border_color(if active {
            theme::accent()
        } else {
            theme::border()
        })
        .text_size(px(12.5))
        .text_color(if active {
            theme::text()
        } else {
            theme::text_muted()
        })
        .cursor_pointer()
        .hover(|this| this.text_color(theme::text()));

    if active {
        row = row.bg(theme::app_bg());
    }

    row.child(status_dot(colour, theme::pane_bg()))
        .child(name)
        .child(
            div()
                .id(eid("agents-tab-close", id))
                .size(px(16.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    Icon::new(IconName::Close)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                // The close benches the agent. It does not end it — see the module note on
                // `ui::agents`.
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new("Put on the bench").build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.bench_agent(id, cx))),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.select_column_tab(column, index, cx)))
        .on_drag(DraggedTab(id), move |_, _, _, cx: &mut App| {
            let ghost = ghost.clone();
            view.update(cx, |this, cx| this.start_tab_drag(id, cx));
            cx.new(|_| TabGhost(ghost))
        })
        .into_any_element()
}

/// The `+` at the end of a strip: which benched agent to group into this column.
///
/// Offered only while there is something to add. A `+` that opens an empty menu is a control that
/// says the bench is empty in the least direct way available.
fn add_tab(app: &AppState, column: usize, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let Some((work, agents)) = app.work(cx).zip(app.agents(cx)) else {
        return div().into_any_element();
    };
    let bench = agents.benched(work);
    if bench.is_empty() {
        return div().into_any_element();
    }
    // Each row names its session as well as the agent. Two sessions may be running an agent by
    // the same name, and a menu that could not tell them apart would open the wrong conversation.
    let names: Vec<String> = bench
        .iter()
        .map(|agent| match work.session(agent.session) {
            Some(session) => format!("{} \u{b7} {}", agent.name, session.name),
            None => agent.name.clone(),
        })
        .collect();
    let ids: Vec<AgentId> = bench.iter().map(|agent| agent.id).collect();

    div()
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .child(
            Picker::new(("agents-add", column), "")
                .icon(IconName::Plus)
                .style(PickerStyle::Chip)
                .items(names)
                .open(app.workbench.open_menu == Some(MenuId::AgentBench(column)))
                .on_toggle(handler(&view, move |this, _, cx| {
                    this.open_menu(MenuId::AgentBench(column), cx)
                }))
                .on_pick(indexed(&view, move |this, index, _, cx| {
                    if let Some(id) = ids.get(index).copied() {
                        this.group_agent_into(column, id, cx);
                    }
                    this.close_menu(cx);
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
        )
        .into_any_element()
}

/// What the agent in front is: its name, its role, what it is doing, and where it is working.
///
/// The second line says how many agents share this column, because a grouped column is drawing one
/// of them and the count is the only thing on screen that says the others are behind it.
fn header(
    agent: &WorkAgent,
    tabs: usize,
    work: &work::WorkProjection,
    colour: gpui::Rgba,
) -> AnyElement {
    let worktree = work
        .session(agent.session)
        .is_some_and(|session| session.worktree);

    let mut place = vec![agent.branch.clone()];
    if worktree {
        place.push("worktree".to_string());
    }
    if tabs > 1 {
        place.push(format!("{tabs} agents grouped"));
    }

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(role_mark(&agent.role, colour, 18.))
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme::text())
                        .child(SharedString::from(agent.name.clone())),
                )
                .child(section_label(&agent.role))
                .child(div().flex_1().min_w(px(0.)))
                .child(state_chip(agent.activity.label(), colour, 1.0)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(IconName::Network)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(mono(place.join(" \u{b7} "), theme::text_muted()).text_size(px(11.5))),
        )
        .into_any_element()
}

/// What has been said to and by this agent, oldest first.
fn thread(app: &AppState, id: AgentId, cx: &mut Context<AppState>) -> AnyElement {
    let Some(agent) = app.work(cx).and_then(|work| work.agent(id)) else {
        return div().into_any_element();
    };

    let turns: Vec<AnyElement> = agent
        .thread
        .iter()
        .map(|turn| match turn.from {
            // What the user said sits in the accent, the way the chat panel draws the same thing.
            Speaker::You => div()
                .pl_6()
                .child(
                    div()
                        .p_2()
                        .bg(theme::accent_soft())
                        .border_l(px(theme::ACCENT_EDGE))
                        .border_color(theme::accent())
                        .text_size(px(13.))
                        .text_color(theme::text())
                        .child(SharedString::from(turn.text.clone())),
                )
                .into_any_element(),
            Speaker::Agent => div()
                .p_2()
                .bg(theme::surface())
                .text_size(px(13.))
                .text_color(theme::text())
                .child(SharedString::from(turn.text.clone()))
                .into_any_element(),
        })
        .collect();

    div()
        .id(eid("agents-thread", id))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .py_2()
        .gap_2()
        .overflow_y_scroll()
        .children(turns)
        .child(
            div()
                .pt_1()
                .text_size(px(11.5))
                .text_color(theme::text_faint())
                .child(
                    "Nothing is listening yet \u{2014} what you send reaches the host and no agent \
                     answers it.",
                ),
        )
        .into_any_element()
}

/// What the host said about the harness behind this column: which one it is, which model, and how
/// much of the context window is gone.
///
/// There is no mode chip. A harness's mode is not on the record, and a chip reading the chat
/// panel's selection would be reporting a setting that has nothing to do with this agent.
fn footer(agent: &WorkAgent) -> AnyElement {
    // Each pill is drawn only where the record has something to put in it. A pill is a box with
    // a border, so an empty string still draws — a small box saying nothing, which reads as a
    // value the interface failed to show rather than one the harness has not reported.
    let mut row = div()
        .px_3()
        .py_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .border_t_1()
        .border_color(theme::border());

    if !agent.harness.is_empty() {
        row = row.child(
            pill(theme::accent())
                .h(px(22.))
                .px_2()
                .child(mono(HARNESS_GLYPH, theme::text()).text_size(px(11.))),
        );
    }
    // Which identity it runs as, chosen when it started and not changeable after.
    if !agent.account.is_empty() {
        row = row.child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono(agent.account.clone(), theme::text_muted()).text_size(px(11.))),
        );
    }
    if !agent.model.is_empty() {
        row = row.child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono(agent.model.clone(), theme::text()).text_size(px(11.))),
        );
    }

    row.child(div().flex_1().min_w(px(0.)))
        .child(progress_ring(agent.context_pct, 12.))
        .child(
            mono(
                format!("{} ctx", work::tokens_label(agent)),
                theme::text_muted(),
            )
            .text_size(px(11.)),
        )
        .into_any_element()
}

/// The field that steers this column, addressed at whatever tab is in front.
///
/// The textarea is the window's — one per column slot, from a fixed pool — and what is typed is
/// mirrored onto the project's drafts by the subscription that owns it. The placeholder names the
/// agent, and is set when the column opens or changes tab.
fn composer(
    app: &AppState,
    column: usize,
    slot: usize,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(input) = app.column_inputs.get(slot).cloned() else {
        return div().into_any_element();
    };
    let can_send = app
        .agents(cx)
        .is_some_and(|agents| !agents.draft(slot).trim().is_empty());
    let focused = input.read(cx).focus_handle(cx).is_focused(window);

    field(theme::accent(), focused)
        .flex_none()
        .flex_col()
        .items_stretch()
        .child(
            div()
                .id(("agents-composer", column))
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
        .child(
            div()
                .px_2()
                .pb_2()
                .pt_1()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    mono(
                        "\u{23ce} send \u{b7} \u{21e7}\u{23ce} newline",
                        theme::text_faint(),
                    )
                    .text_size(px(10.5)),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    ghost_button(
                        ElementId::Name(format!("agents-send-{slot}").into()),
                        Some(IconName::ArrowUp),
                        "Send",
                        cx.listener(move |this, _, window, cx| this.steer_column(slot, window, cx)),
                    )
                    .text_color(if can_send {
                        theme::accent()
                    } else {
                        theme::text_faint()
                    }),
                ),
        )
        .into_any_element()
}

/// What follows the pointer while a tab is being dragged: the agent's name, nothing else. A tab is
/// a name and a dot, and a dot with no column behind it says nothing.
struct TabGhost(SharedString);

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .bg(theme::surface_raised())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::accent())
            .text_size(px(12.5))
            .text_color(theme::text())
            .child(self.0.clone())
    }
}
