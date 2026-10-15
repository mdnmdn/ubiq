//! The new-mission dialog (`Layer::NewMission`): title, requirements, attachments, the tasks to
//! include, an optional initial plan, then the driver, planner, worker and — under *More* —
//! parallel and approve.
//!
//! **No control here is new.** The attachments use the task attachment picker (project and KB
//! roots) and the same clipboard paste the task panel has; *Initial plan → File…* is that same
//! picker answering with one target; the included tasks use the multi-select picker with its own
//! filter field. Start sends one `CreateMission`.
//!
//! Labelled with the project's own word for a mission ([`crate::app::AppState::mission_term`]).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, Focusable, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use gpui_component::input::{Input, Textarea};
use gpui_component::{IconName, Sizable as _};

use crate::app::AppState;
use crate::state::Layer;
use crate::state::new_mission::PlanSource;
use crate::theme;
use crate::ui::kit::menu::{MultiPicker, Picker, PickerStyle};
use crate::ui::kit::{
    check_box, choice_pill, field, ghost_button, hint_row, icon_button, label_hint, modal, mono,
    primary_button, removable_tag,
};
use crate::ui::{handler, indexed};
use ubiq_proto::mission::Driver;

const CONTROL_WIDTH: f32 = 230.;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.new_mission.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let term = app.mission_term(cx);

    let mut body = div().flex().flex_col().gap_3().pt_1();

    body = body.child(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_hint(
                "new-mission-title-hint",
                "Title",
                "What to call this mission. Left blank, it is taken from the first line of the \
                 requirements.",
            ))
            .child(
                field(
                    theme::border(),
                    app.new_mission_title_input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window),
                )
                .h(px(30.))
                .px_2()
                .child(Input::new(&app.new_mission_title_input).appearance(false)),
            ),
    );

    let description_focused = app
        .new_mission_description_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    body = body.child(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_hint(
                "new-mission-description-hint",
                "Requirements",
                "Markdown. Written onto the mission as its description, and carried in the \
                 planner's opening brief.",
            ))
            .child(
                field(theme::border(), description_focused)
                    .flex_col()
                    .items_stretch()
                    .child(
                        div().px_2().py_1p5().cursor_text().child(
                            Textarea::new(&app.new_mission_description_input)
                                .appearance(false)
                                .bordered(false)
                                .w_full()
                                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body)),
                        ),
                    ),
            ),
    );

    body = body.child(attachments(app, &form, cx));
    body = body.child(linked_tasks(app, &form, window, cx));
    body = body.child(initial_plan(app, &form, window, cx));

    body = body.child(div().h(px(1.)).w_full().bg(theme::border()));

    body = body.child(hint_row(
        "new-mission-driver-hint",
        "Driver",
        "Auto starts ready tasks by itself until the next human task or approval. Manual lists \
         them and waits for you to press \u{25b6}. Changeable at any time.",
        div()
            .flex()
            .flex_none()
            .justify_end()
            .items_center()
            .gap_1p5()
            .w(px(CONTROL_WIDTH))
            .child(choice_pill(
                "new-mission-driver-auto",
                "Auto",
                form.driver == Driver::Auto,
                cx.listener(|this, _, _, cx| this.set_new_mission_driver(Driver::Auto, cx)),
            ))
            .child(choice_pill(
                "new-mission-driver-manual",
                "Manual",
                form.driver == Driver::Manual,
                cx.listener(|this, _, _, cx| this.set_new_mission_driver(Driver::Manual, cx)),
            ))
            .into_any_element(),
    ));

    let definitions = app.new_mission_definitions(cx);
    body = body.child(definition_row(
        &view,
        "new-mission-planner",
        "Planner",
        "The agent that writes the plan and drafts the tasks. Left on the project default, it \
         is the definition tagged planner.",
        &definitions,
        form.planner.as_ref(),
        form.planner_open,
        true,
    ));
    body = body.child(definition_row(
        &view,
        "new-mission-worker",
        "Worker",
        "The agent started on each task, unless the task names one of its own.",
        &definitions,
        form.worker.as_ref(),
        form.worker_open,
        false,
    ));

    body = body.child(more(&form, cx));

    let ready = form.ready();
    let footer = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .justify_end()
        .gap_2()
        .child(ghost_button(
            "new-mission-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_new_mission(cx)),
        ))
        .child(
            primary_button(
                "new-mission-start",
                None,
                "Start",
                cx.listener(|this, _, _, cx| this.start_new_mission(cx)),
            )
            .when(!ready, |button| button.opacity(0.5)),
        )
        .into_any_element();

    modal(
        "new-mission",
        theme::accent(),
        &format!("New {}", term.to_lowercase()),
        body.into_any_element(),
        footer,
        crate::ui::dismiss(&view, Layer::NewMission, |this, _, cx| {
            this.close_new_mission(cx)
        }),
        window,
    )
}

/// The attachments row: a chip each, the picker's `+`, and the clipboard.
///
/// **The same two ways in the task panel has** — `raise_new_mission_attachment_picker` opens the
/// file picker over the project tree *and* the knowledge base, and the clipboard control takes
/// what is on the pasteboard, which is a path for a copied file and a picture written into
/// `.ubiq/pasted/` for a screenshot. What is collected here becomes the mission task's own
/// `attachments` (M7), so the chips are the same thing the task panel will draw afterwards.
fn attachments(
    app: &AppState,
    form: &crate::state::new_mission::NewMissionForm,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let chips: Vec<AnyElement> = form
        .attachments
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let open = target.clone();
            let drop = target.clone();
            removable_tag(
                ("new-mission-attachment", index),
                ("new-mission-attachment-drop", index),
                target.clone(),
                format!("Open {target}"),
                theme::surface(),
                theme::text_muted(),
                // A knowledge-base attachment reads in the accent, the one thing about a target
                // its name does not say — the task panel's own rule.
                match open.starts_with("kb:") {
                    true => theme::accent(),
                    false => theme::text_muted(),
                },
                false,
                cx.listener(move |this, _, _, cx| this.open_task_attachment(open.clone(), cx)),
                cx.listener(move |this, _, _, cx| {
                    this.remove_new_mission_attachment(drop.clone(), cx)
                }),
            )
            .into_any_element()
        })
        .collect();
    let empty = form.attachments.is_empty();
    let _ = app;

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(label_hint(
            "new-mission-attachments-hint",
            "Attachments",
            "Project files and knowledge-base documents. Written onto the mission's own task.",
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .children(chips)
                .when(empty, |row| {
                    row.child(mono("nothing attached", theme::text_faint()))
                })
                .child(
                    icon_button(
                        "new-mission-attach",
                        IconName::Plus,
                        false,
                        cx.listener(|this, _, window, cx| {
                            this.raise_new_mission_attachment_picker(window, cx)
                        }),
                    )
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Attach files or KB documents")
                            .build(window, cx)
                    }),
                )
                .child(
                    icon_button(
                        "new-mission-paste",
                        IconName::Copy,
                        false,
                        cx.listener(|this, _, _, cx| this.paste_into_new_mission(cx)),
                    )
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new(
                            "Attach what is on the clipboard \u{2014} a file, or a picture",
                        )
                        .build(window, cx)
                    }),
                ),
        )
        .into_any_element()
}

/// The linked-tasks row: the multi-select picker with its own filter field.
///
/// **The picker's ticks are the chip row.** A second row of chips beside a control that already
/// lists what is picked, and unpicks it on a click, would be the same set twice — so the trigger
/// says what is linked and the list is where one is added or taken off. What is collected becomes
/// the mission's subtasks.
fn linked_tasks(
    app: &AppState,
    form: &crate::state::new_mission::NewMissionForm,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let candidates = app.new_mission_task_candidates(cx);
    let Some(work) = app.work(cx) else {
        return div().into_any_element();
    };
    let labels: Vec<String> = candidates
        .iter()
        .filter_map(|id| work.task(*id))
        .map(|task| match task.key.clone() {
            Some(key) => format!("{key} \u{00b7} {}", task.title),
            None => task.title.clone(),
        })
        .collect();
    let lit: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, id)| form.include.contains(id))
        .map(|(ix, _)| ix)
        .collect();
    let focused = app
        .new_mission_task_query
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let picker = MultiPicker::new("new-mission-tasks", "no tasks included")
        .items(labels)
        .selected(lit)
        .style(PickerStyle::Field)
        .search(&app.new_mission_task_query, focused)
        .above_modal()
        .open(form.tasks_open)
        .on_toggle(handler(&view, |this, _window, cx| {
            this.toggle_new_mission_task_list(cx)
        }))
        .on_dismiss(handler(&view, |this, _window, cx| {
            this.toggle_new_mission_task_list(cx)
        }))
        // Read back by index off the same list that drew it.
        .on_pick(indexed(&view, |this, index, _window, cx| {
            if let Some(id) = this.new_mission_task_candidates(cx).get(index).copied() {
                this.toggle_new_mission_include(id, cx);
            }
        }));

    hint_row(
        "new-mission-tasks-hint",
        "Include tasks",
        "Existing tasks that become this mission's subtasks.",
        div()
            .flex_none()
            .w(px(CONTROL_WIDTH))
            .child(picker)
            .into_any_element(),
    )
}

/// A planner or worker picker: *Project default*, then the project's agent definitions.
#[allow(clippy::too_many_arguments)]
fn definition_row(
    view: &gpui::Entity<AppState>,
    id: &'static str,
    label: &str,
    hint: &str,
    definitions: &[Option<String>],
    held: Option<&String>,
    open: bool,
    planner: bool,
) -> AnyElement {
    let labels: Vec<String> = definitions
        .iter()
        .map(|definition| match definition {
            Some(id) => id.clone(),
            None => "Project default".to_string(),
        })
        .collect();
    let selected = definitions
        .iter()
        .position(|definition| definition.as_ref() == held)
        .unwrap_or(0);
    let trigger = labels
        .get(selected)
        .cloned()
        .unwrap_or_else(|| "Project default".to_string());

    let picker = Picker::new(ElementId::Name(id.into()), trigger)
        .items(labels)
        .selected(selected)
        .style(PickerStyle::Field)
        .above_modal()
        .open(open)
        .on_toggle(handler(view, move |this, _window, cx| match planner {
            true => this.toggle_new_mission_planner_list(cx),
            false => this.toggle_new_mission_worker_list(cx),
        }))
        // Read back by index off the same list that drew it.
        .on_pick(indexed(view, move |this, index, _window, cx| {
            if let Some(picked) = this.new_mission_definitions(cx).get(index).cloned() {
                match planner {
                    true => this.pick_new_mission_planner(picked, cx),
                    false => this.pick_new_mission_worker(picked, cx),
                }
            }
        }));

    hint_row(
        id,
        label,
        hint,
        div()
            .flex_none()
            .w(px(CONTROL_WIDTH))
            .child(picker)
            .into_any_element(),
    )
}

/// *Initial plan*: None, a file through the attachment picker, or pasted text.
///
/// The host copies the content in as `plan.md`'s first revision and the planner is told to refine
/// it rather than write one from scratch.
fn initial_plan(
    app: &AppState,
    form: &crate::state::new_mission::NewMissionForm,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = app
        .new_mission_plan_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let file = match &form.plan_source {
        PlanSource::File(target) => Some(target.clone()),
        _ => None,
    };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(hint_row(
            "new-mission-plan-hint",
            "Initial plan",
            "Optional, a starting point for the planner to refine. A file is copied in as the \
             plan's first revision and stays attached for reference.",
            div()
                .flex()
                .flex_none()
                .justify_end()
                .items_center()
                .gap_1p5()
                .w(px(CONTROL_WIDTH))
                .child(choice_pill(
                    "new-mission-plan-none",
                    "None",
                    form.plan_source == PlanSource::None,
                    cx.listener(|this, _, _, cx| {
                        this.set_new_mission_plan_source(PlanSource::None, cx)
                    }),
                ))
                .child(choice_pill(
                    "new-mission-plan-file",
                    "File\u{2026}",
                    file.is_some(),
                    cx.listener(|this, _, window, cx| {
                        this.raise_new_mission_plan_picker(window, cx)
                    }),
                ))
                .child(choice_pill(
                    "new-mission-plan-paste",
                    "Paste",
                    form.plan_source == PlanSource::Paste,
                    cx.listener(|this, _, _, cx| {
                        this.set_new_mission_plan_source(PlanSource::Paste, cx)
                    }),
                ))
                .into_any_element(),
        ))
        .when_some(file, |row, target| {
            row.child(mono(target, theme::text_muted()))
        })
        .when(form.plan_source == PlanSource::Paste, |row| {
            row.child(
                field(theme::border(), focused)
                    .flex_col()
                    .items_stretch()
                    .child(
                        div().px_2().py_1p5().cursor_text().child(
                            Textarea::new(&app.new_mission_plan_input)
                                .appearance(false)
                                .bordered(false)
                                .w_full()
                                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body)),
                        ),
                    ),
            )
        })
        .into_any_element()
}

/// *More*: parallel and approve, folded away until asked for.
fn more(
    form: &crate::state::new_mission::NewMissionForm,
    cx: &mut Context<AppState>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .id("new-mission-more")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.toggle_new_mission_more(cx)))
                .child(
                    gpui_component::Icon::new(match form.more_open {
                        true => IconName::ChevronDown,
                        false => IconName::ChevronRight,
                    })
                    .with_size(gpui_component::Size::XSmall)
                    .text_color(theme::text_muted()),
                )
                .child(mono("More", theme::text_muted())),
        )
        .when(form.more_open, |col| {
            col.child(hint_row(
                "new-mission-parallel-hint",
                "Parallel",
                "How many workers may run at once.",
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .items_center()
                    .gap_1p5()
                    .w(px(CONTROL_WIDTH))
                    .child(choice_pill(
                        "new-mission-parallel-down",
                        "\u{2212}",
                        false,
                        cx.listener(|this, _, _, cx| this.nudge_new_mission_parallel(false, cx)),
                    ))
                    .child(mono(form.parallel.to_string(), theme::text()))
                    .child(choice_pill(
                        "new-mission-parallel-up",
                        "+",
                        false,
                        cx.listener(|this, _, _, cx| this.nudge_new_mission_parallel(true, cx)),
                    ))
                    .into_any_element(),
            ))
            .child(hint_row(
                "new-mission-approve-hint",
                "Approve each task",
                "Default for new subtasks: a finished task waits for your Confirm before its \
                 dependents start.",
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .w(px(CONTROL_WIDTH))
                    .child(check_box(
                        "new-mission-approve",
                        form.approve,
                        cx.listener(|this, _, _, cx| this.toggle_new_mission_approve(cx)),
                    ))
                    .into_any_element(),
            ))
        })
        .into_any_element()
}
