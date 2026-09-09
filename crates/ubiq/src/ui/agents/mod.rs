//! The Agents screen: every agent this window holds a live conversation for, listed down one side,
//! and the ones the user is working with drawn as **parallel columns** of conversation across the
//! rest.
//!
//! **Not everything the host reports.** The projection is wider than what a column can talk to —
//! it carries the mock work thread's fixtures too — so every reader on this screen goes through
//! [`crate::state::AgentsView::live_agents`]. The Teams screen keeps reading the whole projection,
//! because a graph is a map of who spawned whom and a mock has a place on one.
//!
//! This is the screen for *talking to* the agents. The screen for *arranging* them is
//! [`crate::ui::orchestration`], and the two never share a view: a graph is a map of who spawned
//! whom, a column is a transcript and a composer.
//!
//! Three things are on screen and each answers one question. The **sidebar** answers *what is
//! running* — every session, every agent in it, what each is doing, and which of them is on the
//! bench. A **column** answers *what is this one saying* — one agent in front, its harness and its
//! context, its thread, and a field that steers it. The **strip between them** answers *how the
//! columns are filled*, and says how to change it.
//!
//! **A column holds tabs, and more than one tab is a group.** Dragging a tab onto another column
//! puts the two agents in one strip, which is how the user reads a hand-off — the plan and the
//! build side by side, one column wide. Dragging it past the last column gives it a column of its
//! own again.
//!
//! **Closing a tab benches the agent; it does not end it.** That is the one place this screen
//! deliberately reads differently from a terminal pane, whose close kills the harness behind it. A
//! tab is a view onto a conversation, so taking it off screen leaves the agent running — the
//! sidebar still lists it, marked `bench`, and one click brings it back. Nothing here kills an
//! agent.
//!
//! The records are the host's, projected into [`crate::state::work`]; the arrangement over them is
//! this window's, in [`crate::state::agents`], and no message carries it. Three files: the list is
//! [`sidebar`], one column is [`column`], and this module is the frame.

pub mod column;
pub mod sidebar;

use gpui::{
    AnyElement, Context, Focusable as _, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::NewAgentSurface;
use crate::theme;
use crate::ui::empty;
use crate::ui::kit::{self, ghost_button, mono};
use crate::ui::{handler, indexed};

/// What a dragged tab carries. The agent alone: which column it came from is a question the view
/// can already answer, and the drop cares only about where it landed.
#[derive(Clone, Debug)]
pub struct DraggedTab(pub ubiq_proto::work::AgentId);

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The screen is a view of one project's work, and the shell keeps a window with no project off
    // it entirely — so there is nothing here to draw rather than an empty row to explain.
    let (Some(work), Some(agents)) = (app.work(cx), app.agents(cx)) else {
        return div().into_any_element();
    };

    let field = if agents.columns.is_empty() {
        // Every agent on the bench. The page says which control puts one back rather than leaving
        // an empty row that reads as a project with nothing running.
        let note = if agents.live_agents(work).is_empty() {
            "Nothing is running in this project yet."
        } else {
            "Every agent is on the bench. Pick one in the list to open a column."
        };
        empty::empty_page("No columns", note, IconName::Asterisk, None).into_any_element()
    } else {
        columns(app, window, cx)
    };

    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(
            div()
                .w(px(theme::AGENT_SIDEBAR_WIDTH))
                .flex()
                .flex_none()
                .border_r_1()
                .border_color(theme::border())
                .child(sidebar::render(app, cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(header(app, cx))
                .child(field),
        )
        .into_any_element()
}

/// The strip over the columns: how they are filled, and how to change it.
///
/// The hint is on screen rather than in a tooltip because the two gestures it names are the only
/// way to group and ungroup, and neither leaves a mark on the interface to be discovered from.
fn header(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(agents) = app.agents(cx) else {
        return div().into_any_element();
    };

    div()
        .h(px(theme::TITLEBAR_HEIGHT))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(mono(
            format!(
                "{} columns \u{b7} {} agents \u{b7} {} grouped",
                agents.columns.len(),
                agents.on_the_field(),
                agents.grouped()
            ),
            theme::text_muted(),
        ))
        .child(div().flex_1().min_w(px(0.)))
        .child(
            mono(
                "drag a tab onto another column to group \u{b7} drop right to open a new one",
                theme::text_faint(),
            )
            .text_size(px(11.5)),
        )
        .children(close_all(app, cx))
        .child(new_agent(cx))
        .into_any_element()
}

/// **Close all**: benches every agent on screen, through
/// [`AppState::close_all_conversations`] — the same thing a tab's own close does, for the whole
/// row at once. Nothing is ended; see the module doc. Offered only when there is something on
/// screen to bench — a row with no columns gets no button rather than one that would silently
/// no-op.
fn close_all(app: &AppState, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let has_columns = app
        .agents(cx)
        .is_some_and(|agents| !agents.columns.is_empty());
    if !has_columns {
        return None;
    }
    Some(
        ghost_button(
            "agents-close-all",
            Some(IconName::Close),
            "Close all",
            cx.listener(|this, _, _, cx| this.close_all_conversations(cx)),
        )
        .text_color(theme::danger())
        .into_any_element(),
    )
}

/// **New agent**: the `+` that asks what this screen should be showing. It opens
/// [`new_agent_menu`], which the shell paints.
fn new_agent(cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .child(ghost_button(
            "agents-new",
            Some(IconName::Plus),
            "New agent",
            cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                let at = event.position();
                this.open_new_agent_menu((at.x.into(), at.y.into()), NewAgentSurface::Agents, cx);
            }),
        ))
        .into_any_element()
}

/// The menu every `+` in the window opens, at the point that was clicked.
///
/// **Two rows, because there are two questions.** *New agent* raises the form, which asks the
/// harness, the identity, the model, the level and the mode together; *Attach existing agent*
/// lists the conversations this project already has. The flattened harness-and-identity list this
/// menu used to be is gone: it started a conversation with every question but the first skipped,
/// and the form is what asks them.
///
/// The second row is the second *stage* of the same menu rather than a submenu — the kit has
/// none, and a list that can run to every conversation in the project does not belong under a row
/// that is not it.
///
/// **The rows and the pick are one list**, matched by position: the first stage's two rows are
/// fixed, and the second's are [`AppState::attach_rows`], read again by
/// [`AppState::pick_new_agent_menu`] exactly as they were drawn.
///
/// **The second stage is a [`kit::Picker`], not a context menu**, because it can run to every
/// conversation in the project and a list that long has to be searchable. The chat header's
/// chevron has always been one; this is the same mechanism, and the window keeps exactly one way
/// to narrow a list. The picker is anchored at the click point by an absolutely-placed trigger,
/// which its own panel then covers — the first stage keeps the context menu, whose two fixed rows
/// have nothing to filter.
///
/// Painted by [`crate::ui::shell`] rather than from here, because three surfaces open it — this
/// screen's control, the IDE chat strip's `+` and the sink's bench — and the state it reads is the
/// window's, not any page's.
pub fn new_agent_menu(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let Some(menu) = app.workbench.new_agent_menu else {
        return div().into_any_element();
    };

    if menu.attach {
        let rows = app.attach_rows(menu.surface, cx);
        let search_focused = app
            .picker_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        // No label and no icon: the trigger is only what the panel hangs from, and the panel
        // covers it.
        let picker = kit::Picker::new("agents-attach-menu", "")
            .items(rows.items.iter().map(|(_, name)| name.clone()))
            // Already shown by another panel of this surface: drawn, not dropped — a row that
            // vanishes reads as a conversation that ended rather than one taken.
            .disabled(rows.disabled.clone())
            .open(true)
            .search(&app.picker_search, search_focused)
            .on_pick(indexed(&view, |this, index, window, cx| {
                this.pick_new_agent_menu(index, window, cx);
            }))
            .on_dismiss(handler(&view, |this, _, cx| {
                this.dismiss_new_agent_menu(cx)
            }));
        return div()
            .absolute()
            .left(px(menu.at.0))
            .top(px(menu.at.1))
            .child(picker)
            .into_any_element();
    }

    let nothing_to_attach = app.attach_rows(menu.surface, cx).items.is_empty();
    let attach = kit::ContextItem::new("Attach existing agent");
    let items: Vec<kit::ContextItem> = vec![
        kit::ContextItem::new("New agent"),
        if nothing_to_attach {
            attach.disabled()
        } else {
            attach
        },
    ];

    kit::context_menu(
        "agents-new-menu",
        point(px(menu.at.0), px(menu.at.1)),
        items,
        indexed(&view, |this, index, window, cx| {
            this.pick_new_agent_menu(index, window, cx);
        }),
        handler(&view, |this, _, cx| this.dismiss_new_agent_menu(cx)),
    )
    .into_any_element()
}

/// The row of columns, and the strip past the last one that a dragged tab is split off into.
fn columns(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(agents) = app.agents(cx) else {
        return div().into_any_element();
    };
    let count = agents.columns.len();

    let drawn: Vec<AnyElement> = (0..count)
        .map(|ix| column::render(app, ix, window, cx))
        .collect();

    div()
        .id("agents-columns")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_x_scroll()
        .children(drawn)
        .child(new_column_strip(app, cx))
        .into_any_element()
}

/// The narrow strip at the end of the row: where a tab is dropped to get a column of its own.
///
/// It is only a drop target — there is no agent to open here without one being dragged, and the
/// sidebar is where a benched agent is brought on. So it draws as a hairline with a mark on it
/// while something is in the air, and as nothing the rest of the time.
///
/// It lights up only for a drop that would do something: a tab already alone in its column is
/// already what this strip produces, and a full row has no ninth column to give. A target that
/// promises a change it will not make is worse than one that does not light.
fn new_column_strip(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let carrying = app.agents(cx).is_some_and(|agents| {
        agents.has_room()
            && agents.dragging.is_some_and(|dragged| {
                agents
                    .holds(dragged)
                    .and_then(|(col, _)| agents.columns.get(col))
                    .is_some_and(|column| column.grouped())
            })
    });

    let mut strip = div()
        .id("agents-new-column")
        .w(px(theme::NEW_COLUMN_STRIP))
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(theme::border())
        .on_drop(cx.listener(|this, _: &DraggedTab, _, cx| this.drop_tab_at_end(cx)));

    if carrying {
        strip = strip.bg(theme::accent_soft()).child(
            Icon::new(IconName::Plus)
                .with_size(Size::XSmall)
                .text_color(theme::accent()),
        );
    }

    strip.into_any_element()
}
