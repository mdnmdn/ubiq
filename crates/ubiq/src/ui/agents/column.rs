//! One column of the agents screen: the tabs it holds, the agent in front, its thread, and the
//! field that steers it.
//!
//! A column is the whole of one conversation in one place, which is why it wears its own chrome
//! rather than borrowing the chat panel's: the tabs are agents, the header says what this one *is*
//! rather than which chat is open, and the footer reports what the host said about the harness
//! behind it. The left edge takes the active agent's activity colour, so a row of columns reads as
//! a row of states from across the window.
//!
//! **The state hexagon is on every tab, and nowhere else (T-99/T-102).** It used to also sit
//! beside the title, a line above the lifecycle strip's own reading of the same fact — two marks
//! for one state, a line apart. `crate::ui::teams::status::status_mark` draws it: the outer
//! hexagon is the lifecycle, the inner fill the activity or the result, gray through and through
//! once a delegate — or here, an agent's own record — is done rather than merely stopped. The
//! current-action chip beside the three-dots menu, on the line [`crate::ui::conversation`]'s
//! shared header draws, is what a reader scanning for *what* an agent is doing reads now; the
//! hexagon on the tab is *whether* it still can.
//!
//! **A tab is dragged, not reordered.** Dropped on another column it groups; dropped past the last
//! one it splits off. Both are the same gesture from the user's side, and neither sends anything —
//! the arrangement is this window's own.
//!
//! Nothing here writes into a transcript. What is typed reaches the host and the line appears in
//! the thread when the host answers with the agent carrying it: an interface that draws its own
//! half of a conversation is inventing the other half too.
//!
//! **A column draws one thing below its header, and it is the chat panel (T-143).**
//! [`crate::ui::conversation`] is the one view every surface that shows a conversation shares —
//! the chat panel the Teams and IDE screens dock, the kitchen sink, and this. A column passes a
//! different [`ConversationView`] and nothing else; it does not own a transcript, a footer or a
//! composer of its own. It used to carry a second set of all three for an agent that was a record
//! and nothing more, which no column could reach: `AgentsView::live` is the conversations this
//! window holds, and a tab not in it is pruned.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::{AgentId, WorkAgent};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::agents::{BenchRow, COLUMN_MIN_WIDTH};
use crate::state::work;
use crate::theme;
use crate::ui::agents::DraggedTab;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::kit::{Picker, PickerStyle, mono, section_label};
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
        .border_l(px(theme::accent_edge()))
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
        .child(add_tab(app, column, window, cx));

    let root = root
        .child(strip)
        .child(header(app, agent, held.tabs.len(), work, colour));

    // The one chat panel, the same one a chat tab docks on the Teams and IDE screens. What a
    // column changes is the `ConversationView` it hands over: its own id prefix, its own composer
    // slot, and `header: true` — the chat tab draws the same lifecycle row from its own toolbar
    // instead, so that it can put a chevron on it.
    //
    // A column whose agent has no conversation is a frame out of date, not a state: the tab is
    // pruned on the next `WorkList`. So it says that, rather than drawing a second transcript.
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
                    header: true,
                },
                window,
                cx,
            ))
            .into_any_element(),
        None => root
            .child(crate::ui::empty::empty_page(
                "No conversation",
                "This window is not holding a conversation for this agent any more.",
                IconName::CircleX,
                None,
            ))
            .into_any_element(),
    }
}

/// One tab: a state hexagon, the agent's name, and the close that benches it.
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
    let name = app.agent_title(agent);
    let ghost = name.clone();
    // What the conversation is about, where something has named it. A tab with no summary says
    // nothing on hover: the name is printed in full beside the mark already.
    let summary: Option<SharedString> = agent.summary.clone().map(SharedString::from);
    // The same reading the title carries, so a grouped column's tabs and its title agree. A
    // record with no live conversation behind it reads `agent_status`'s own record fallback
    // rather than nothing — the hexagon draws whatever the record can say either way.
    let status = crate::state::status::agent_status(agent, app.conversation(id, cx));

    let mut row = div()
        .id(eid("agents-tab", id))
        .h(px(32.))
        .px_2()
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
        .text_size(theme::font(theme::Family::Conversation, theme::Role::Body))
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

    row.child(crate::ui::teams::status::status_mark(
        status,
        14.0,
        eid("agents-tab-mark", id),
    ))
    .child(
        div()
            .id(eid("agents-tab-name", id))
            .child(name)
            .when_some(summary, |this, summary| {
                this.tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(summary.clone()).build(window, cx)
                })
            }),
    )
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

/// The `+` at the end of a strip: which agent to group into this column — from the bench, or
/// already on screen in some other column, where it draws disabled rather than dropped from the
/// list (see [`crate::state::agents::AgentsView::open_in`], which never leaves an agent in two
/// columns).
///
/// Offered only while there is something to add at all — checked unfiltered, so narrowing a
/// search to nothing does not make the control itself vanish. A `+` that opens an empty menu is a
/// control that says the bench is empty in the least direct way available.
fn add_tab(
    app: &AppState,
    column: usize,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let Some((work, agents)) = app.work(cx).zip(app.agents(cx)) else {
        return div().into_any_element();
    };
    if agents.bench_rows(column, work, "").is_empty() {
        return div().into_any_element();
    }

    let query = app.picker_search.read(cx).value().to_string();
    let search_focused = app
        .picker_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let rows = agents.bench_rows(column, work, &query);

    // Each agent row names its session as well as the agent. Two sessions may be running an
    // agent by the same name, and a menu that could not tell them apart would open the wrong
    // conversation.
    let names: Vec<String> = rows
        .iter()
        .map(|row| match row {
            BenchRow::Agent { id, .. } => match work.agent(*id) {
                Some(agent) => match work.session(agent.session) {
                    Some(session) => format!("{} \u{b7} {}", agent.name, session.name),
                    None => agent.name.clone(),
                },
                None => String::new(),
            },
            BenchRow::Label(text) => text.to_string(),
            BenchRow::Separator => String::new(),
        })
        .collect();
    let disabled: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(ix, row)| match row {
            BenchRow::Agent { disabled: true, .. } | BenchRow::Label(_) => Some(ix),
            _ => None,
        })
        .collect();
    let separators: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(ix, row)| matches!(row, BenchRow::Separator).then_some(ix))
        .collect();

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
                .disabled(disabled)
                .separators(separators)
                .search(&app.picker_search, search_focused)
                .open(app.workbench.open_menu == Some(MenuId::AgentBench(column)))
                .on_toggle(handler(&view, move |this, window, cx| {
                    this.open_agent_bench_menu(column, window, cx)
                }))
                .on_pick(indexed(&view, move |this, index, window, cx| {
                    this.pick_agent_bench_menu(column, index, window, cx);
                }))
                .on_dismiss(handler(&view, move |this, window, cx| {
                    this.dismiss_agent_bench_menu(window, cx)
                })),
        )
        .into_any_element()
}

/// What the agent in front is: its name, its role, and where it is working.
///
/// The second line says how many agents share this column, because a grouped column is drawing one
/// of them and the count is the only thing on screen that says the others are behind it.
///
/// **No state mark and no chip here any more (T-102).** The hexagon that used to sit beside the
/// name moved to the tab strip's own tab, where the dot already lived too — one state reading is
/// the same rule [`conversation::lifecycle_header`]'s own doc follows — and the activity chip is
/// [`conversation::lifecycle_header`]'s, drawn on the line below by [`crate::ui::conversation`]'s
/// shared render whenever `view.header` is set, so this row does not say the same fact twice.
fn header(
    app: &AppState,
    agent: &WorkAgent,
    tabs: usize,
    work: &work::WorkProjection,
    colour: gpui::Rgba,
) -> AnyElement {
    let worktree = work
        .session(agent.session)
        .is_some_and(|session| session.worktree);

    let summary: Option<SharedString> = agent.summary.clone().map(SharedString::from);

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
                // The title says which conversation this is; the summary on hover says what it
                // is about, and a conversation nothing has named has nothing to add.
                .child(
                    div()
                        .id(eid("agents-header-name", agent.id))
                        .text_size(theme::font(theme::Family::Conversation, theme::Role::Title))
                        .text_color(theme::text())
                        .child(app.agent_title(agent))
                        .when_some(summary, |this, summary| {
                            this.tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(summary.clone())
                                    .build(window, cx)
                            })
                        }),
                )
                .child(section_label(&agent.role))
                .child(div().flex_1().min_w(px(0.))),
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
                .child(
                    mono(place.join(" \u{b7} "), theme::text_muted())
                        .text_size(theme::font(theme::Family::Conversation, theme::Role::Label)),
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
            .border_l(px(theme::accent_edge()))
            .border_color(theme::accent())
            .text_size(theme::font(theme::Family::Conversation, theme::Role::Body))
            .text_color(theme::text())
            .child(self.0.clone())
    }
}
