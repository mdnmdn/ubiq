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

use ubiq_proto::work::TaskRecord;

use crate::app::AppState;
use crate::state::TeamsSelection;
use crate::state::work::WorkProjection;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{disclosure, mono, slab, tag};
use crate::ui::work::activity_colour;
use crate::ui::{eid, eid2};

/// The drawer under the graph: shut, it is one line saying what there is; open, it is the list.
pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(graph)) = (app.teams_work(cx), app.teams(cx)) else {
        return div().into_any_element();
    };
    let tasks = graph.listed_tasks(&work);
    let steps: usize = tasks.iter().map(|t| t.steps.len()).sum();

    let about = match graph.agent_in_focus() {
        Some(id) => work
            .agent(id)
            .map(|a| app.agent_label(a).title.to_string())
            .unwrap_or_else(|| "\u{2014}".to_string()),
        _ => graph
            .active_session(&work)
            .and_then(|id| work.session(id))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "\u{2014}".to_string()),
    };

    let summary = div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(mono(about, theme::text()))
        .child(
            mono("\u{b7}", theme::text_faint()).text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .child(mono(
            format!("{} tasks \u{b7} {steps} steps", tasks.len()),
            theme::text_muted(),
        ))
        .into_any_element();

    let mut root = div().flex().flex_none().flex_col().child(disclosure(
        "teams-tasks",
        "Tasks",
        summary,
        graph.tasks_open,
        cx.listener(|this, _, _, cx| this.toggle_teams_tasks_drawer(cx)),
    ));

    if graph.tasks_open {
        root = root.child(
            div()
                .h(px(theme::tasks_height()))
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
    let (Some(work), Some(graph)) = (app.teams_work(cx), app.teams(cx)) else {
        return div().into_any_element();
    };
    let tasks = graph.listed_tasks(&work);
    // Unnarrowed — see `AppState::teams_all_tasks` — because `work.tasks` here has already been
    // cut down to what a live agent holds, and a prerequisite outside that cut must still count.
    let all_tasks = app.teams_all_tasks(cx);

    if tasks.is_empty() {
        return div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(theme::text_faint())
                    .child("No task for this selection."),
            )
            .into_any_element();
    }

    let rows: Vec<AnyElement> = tasks
        .iter()
        .map(|task| task_card(&work, &all_tasks, task, cx))
        .collect();

    div()
        .id("teams-task-list")
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

/// One task's card: its shape, whether it waits on something, its title, its steps and their
/// owners.
fn task_card(
    work: &WorkProjection,
    all_tasks: &[TaskRecord],
    task: &TaskRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let done = task.done();
    let total = task.steps.len();
    // Not ready, M20's derived readiness — not `Status::Blocked`, `board::mod::task_card`'s
    // own distinction, drawn the same muted way here.
    let waiting = task.waiting_on(all_tasks);

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
                        .text_size(theme::font(Family::Chrome, Role::Body))
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
                            mono(
                                crate::state::work::AgentLabel::of(owner, None).title,
                                theme::text_muted(),
                            )
                            .text_size(theme::font(Family::Chrome, Role::Meta)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_in_teams(TeamsSelection::Agent(id), cx)
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
            // Nothing at all for a task nobody has shaped, on the rule `Priority::Normal`
            // follows: an absent claim is drawn as absent, not as a word saying so.
            .children(task.shape.map(|shape| {
                mono(shape.label(), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Micro))
            }))
            .children((!waiting.is_empty()).then(|| {
                let keys: Vec<String> = waiting
                    .iter()
                    .filter_map(|id| all_tasks.iter().find(|t| t.id == *id))
                    .map(|other| other.key.clone().unwrap_or_else(|| other.title.clone()))
                    .collect();
                tag(
                    eid("teams-task-waits-on", task.id),
                    format!("waits on {}", waiting.len()),
                    format!("waits on {}", keys.join(", ")),
                    theme::warning_soft(),
                    theme::warning(),
                    theme::warning(),
                    false,
                    |_, _, cx| cx.stop_propagation(),
                )
            }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(if waiting.is_empty() {
                        theme::text()
                    } else {
                        theme::text_muted()
                    })
                    .child(SharedString::from(task.title.clone())),
            )
            .child(
                mono(format!("{done}/{total}"), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            ),
    )
    .children(steps)
    .into_any_element()
}
