//! The tasks belonging to whatever is selected — as a drawer under the graph, and as the body of
//! the inspector's second tab.
//!
//! One list, two places, because it is one question asked at two scales. A session lists every
//! task in it; an agent lists the tasks it has a step in. Every step names its owner, and clicking
//! that owner selects the agent — which is the way back from a task to the workspace doing it.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::Selection;
use crate::theme;
use crate::ui::eid2;
use crate::ui::kit::{disclosure, mono, slab};
use crate::ui::work::activity_colour;

/// The drawer under the graph: shut, it is one line saying what there is; open, it is the list.
pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(graph)) = (app.work(cx), app.graph(cx)) else {
        return div().into_any_element();
    };
    let tasks = graph.listed_tasks(work);
    let steps: usize = tasks.iter().map(|t| t.steps.len()).sum();

    let about = match graph.selection {
        Some(Selection::Agent(id)) => work
            .agent(id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "\u{2014}".to_string()),
        _ => graph
            .active_session(work)
            .and_then(|id| work.session(id))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "\u{2014}".to_string()),
    };

    let summary = div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(mono(about, theme::text()).text_size(px(11.5)))
        .child(mono("\u{b7}", theme::text_faint()).text_size(px(11.)))
        .child(
            mono(
                format!("{} tasks \u{b7} {steps} steps", tasks.len()),
                theme::text_muted(),
            )
            .text_size(px(11.5)),
        )
        .into_any_element();

    let mut root = div().flex().flex_none().flex_col().child(disclosure(
        "orch-tasks",
        "Tasks",
        summary,
        graph.tasks_open,
        cx.listener(|this, _, _, cx| this.toggle_tasks_drawer(cx)),
    ));

    if graph.tasks_open {
        root = root.child(
            div()
                .h(px(theme::TASKS_HEIGHT))
                .flex()
                .flex_none()
                .bg(theme::pane_bg())
                .child(list(app, cx)),
        );
    }

    root.into_any_element()
}

/// The list itself. Used by the drawer and by the inspector's tasks tab, which is why it fills
/// whatever it is put in rather than sizing itself.
pub fn list(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let (Some(work), Some(graph)) = (app.work(cx), app.graph(cx)) else {
        return div().into_any_element();
    };
    let tasks = graph.listed_tasks(work);

    if tasks.is_empty() {
        return div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme::text_faint())
                    .child("No task for this selection."),
            )
            .into_any_element();
    }

    let rows: Vec<AnyElement> = tasks
        .into_iter()
        .map(|task| {
            let done = task.done();
            let total = task.steps.len();

            let steps: Vec<AnyElement> = task
                .steps
                .iter()
                .map(|step| {
                    let owner = step.owner.and_then(|id| work.agent(id));
                    let colour = owner
                        .map(|a| activity_colour(a.activity))
                        .unwrap_or_else(theme::text_faint);

                    let mut row = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .py_1()
                        .child(
                            Icon::new(if step.done() {
                                IconName::CircleCheck
                            } else {
                                IconName::Dash
                            })
                            .with_size(Size::XSmall)
                            .text_color(if step.done() {
                                theme::success()
                            } else {
                                theme::text_faint()
                            }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(12.5))
                                .text_color(if step.done() {
                                    theme::text_muted()
                                } else {
                                    theme::text()
                                })
                                .child(SharedString::from(step.title.clone())),
                        );

                    // The owner is the way back from a task to the workspace doing it.
                    if let Some(owner) = owner {
                        let id = owner.id;
                        row = row.child(
                            div()
                                .id(eid2("task-owner", task.id, step.id))
                                .px_1p5()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .cursor_pointer()
                                .hover(|this| this.bg(theme::hover()))
                                .child(div().size(px(6.)).flex_none().rounded_full().bg(colour))
                                .child(
                                    mono(owner.name.clone(), theme::text_muted())
                                        .text_size(px(11.)),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_in_graph(Selection::Agent(id), cx)
                                })),
                        );
                    }

                    row.into_any_element()
                })
                .collect();

            slab(if done == total {
                theme::success()
            } else {
                theme::accent()
            })
            .p_3()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(mono(task.shape.label(), theme::text_faint()).text_size(px(9.5)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(13.))
                            .text_color(theme::text())
                            .child(SharedString::from(task.title.clone())),
                    )
                    .child(mono(format!("{done}/{total}"), theme::text_muted()).text_size(px(11.))),
            )
            .children(steps)
            .into_any_element()
        })
        .collect();

    div()
        .id("orch-task-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .p_3()
        .gap_2()
        .overflow_y_scroll()
        .children(rows)
        .into_any_element()
}
