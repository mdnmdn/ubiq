//! The Agents screen: every agent the host reports, listed down one side, and the ones the user is
//! working with drawn as **parallel columns** of conversation across the rest.
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
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
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
        let note = if work.agents.is_empty() {
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
        .px_3()
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
        .child(new_agent(app, cx))
        .into_any_element()
}

/// **New agent**: which harness to start a live conversation on, here, in this project.
///
/// The list is the host's own — the same [`AgentTypeInfo`] the new-pane menu offers, asked for
/// once and read by both — and a harness the host could not find on disk is listed and disabled,
/// exactly as it is there. What a pick starts is a conversation rather than a pane: the two are
/// the same question asked of different halves of a workspace, and a conversation has no size.
///
/// [`AgentTypeInfo`]: ubiq_proto::messages::AgentTypeInfo
fn new_agent(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();

    let mut control = div().flex().flex_none().items_center().child(ghost_button(
        "agents-new",
        Some(IconName::Plus),
        "New agent",
        cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
            let at = event.position();
            this.open_new_agent_menu((at.x.into(), at.y.into()), cx);
        }),
    ));

    if app.workbench.new_agent_menu.is_some() {
        let items: Vec<kit::ContextItem> = app
            .workbench
            .agent_types
            .iter()
            .map(|agent| {
                let item = kit::ContextItem::new(SharedString::from(agent.label.clone()));
                if agent.available {
                    item
                } else {
                    item.disabled()
                }
            })
            .collect();
        // Nothing found on this machine is said in the menu rather than by a control that opens
        // on emptiness.
        let items = if items.is_empty() {
            vec![kit::ContextItem::new("No harness found here").disabled()]
        } else {
            items
        };
        let at = app.workbench.new_agent_menu.unwrap_or_default();

        control = control.child(kit::context_menu(
            "agents-new-menu",
            point(px(at.0), px(at.1)),
            items,
            indexed(&view, |this, index, _, cx| {
                this.pick_new_agent_menu(index, cx);
            }),
            handler(&view, |this, _, cx| this.dismiss_new_agent_menu(cx)),
        ));
    }

    control.into_any_element()
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
