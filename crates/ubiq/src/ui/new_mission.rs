//! The new-mission dialog — §5's table, widened rather than replaced.
//!
//! Seven rows over one draft: the title, the **requirements** editor, the attachments, the linked
//! tasks, the plan gate, the coordinator, and an optional plan to start from. The first four are
//! the brief, and the brief **is the anchor task's own fields** (M7) — so none of them invents a
//! message: the dialog collects, and `AppState::settle_new_mission` writes them with `UpdateTask`
//! and `SetTaskField` once the host has minted an id.
//!
//! **No control here is new.** The attachments use the task attachment picker (project and KB
//! roots) and the same clipboard paste the task panel has; the linked tasks use the multi-select
//! picker with its own filter field; the coordinator uses the picker it always did, with the
//! running agents appended to the list it offers (M10).
//!
//! Labelled with the project's own word for a mission ([`crate::app::AppState::mission_term`]), so
//! a team that renamed it to "epic" sees "New epic" both on the board's button and here.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, Focusable, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use gpui_component::IconName;
use gpui_component::input::{Input, Textarea};

use crate::app::AppState;
use crate::state::Layer;
use crate::state::new_mission::coordinator_label;
use crate::theme;
use crate::ui::kit::menu::{MultiPicker, Picker, PickerStyle};
use crate::ui::kit::{
    check_box, field, ghost_button, hint_row, icon_button, label_hint, modal, mono, primary_button,
    removable_tag,
};
use crate::ui::{handler, indexed};

const CONTROL_WIDTH: f32 = 230.;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.new_mission.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let term = app.mission_term(cx);
    // Profiles fit to run one, then the agents already running here — `profiles_in` rather than
    // the global list alone, because a profile scoped to this project is on offer too (G331).
    let candidates = app.new_mission_coordinators(cx);

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
                 coordinator's opening briefing.",
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

    body = body.child(hint_row(
        "new-mission-require-plan-hint",
        "Require plan",
        "Makes moving to in progress the user's own gate: it is refused while the plan is empty \
         or has open threads.",
        div()
            .flex()
            .flex_none()
            .justify_end()
            .w(px(CONTROL_WIDTH))
            .child(check_box(
                "new-mission-require-plan",
                form.require_plan,
                cx.listener(|this, _, _, cx| this.toggle_new_mission_require_plan(cx)),
            ))
            .into_any_element(),
    ));

    // The picker degrades to a sentence rather than an empty list when this build offers no
    // assistant: an empty picker reads as a bug, and this says what to do about it instead.
    if candidates.is_empty() {
        body = body.child(
            div()
                .p_2()
                .border_1()
                .border_color(theme::border())
                .text_color(theme::text_muted())
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .child(
                    "No profile is marked as a mission assistant yet, and no agent is running. \
                     Tick \u{201c}Mission assistant\u{201d} on a profile in the profile editor to \
                     offer it here.",
                ),
        );
    } else {
        let labels: Vec<String> = candidates.iter().map(|(label, _)| label.clone()).collect();
        let selected = form
            .coordinator
            .as_ref()
            .and_then(|held| candidates.iter().position(|(_, row)| row == held));
        let label = coordinator_label(&form, &candidates, "Choose a coordinator\u{2026}");

        let mut picker = Picker::new(ElementId::Name("new-mission-assistant".into()), label)
            .items(labels)
            .style(PickerStyle::Field)
            .above_modal()
            .open(form.open);
        if let Some(selected) = selected {
            picker = picker.selected(selected);
        }
        picker = picker
            .on_toggle(handler(&view, |this, _window, cx| {
                this.toggle_new_mission_assistant_list(cx)
            }))
            // The list is read again here, exactly as it was drawn — the rule every
            // position-matched menu in this window follows.
            .on_pick(indexed(&view, move |this, index, _window, cx| {
                if let Some((_, picked)) = this.new_mission_coordinators(cx).get(index).cloned() {
                    this.pick_new_mission_coordinator(picked, cx);
                }
            }));

        body = body.child(hint_row(
            "new-mission-assistant-hint",
            "Coordinator",
            "One role (M10): a profile ticked as a mission assistant, or an agent already running \
             here, adopted as it is.",
            div()
                .flex_none()
                .w(px(CONTROL_WIDTH))
                .child(picker)
                .into_any_element(),
        ));
    }

    body = body.child(plan_seed(app, &form, window, cx));

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
/// `.ubiq/pasted/` for a screenshot. What is collected here becomes the anchor task's own
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
/// the anchor's `references` (M7).
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
        .filter(|(_, id)| form.references.contains(id))
        .map(|(ix, _)| ix)
        .collect();
    let focused = app
        .new_mission_task_query
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let picker = MultiPicker::new("new-mission-tasks", "no tasks linked")
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
                this.toggle_new_mission_reference(id, cx);
            }
        }));

    hint_row(
        "new-mission-tasks-hint",
        "Linked tasks",
        "The tasks this mission names. Written onto its own task as references.",
        div()
            .flex_none()
            .w(px(CONTROL_WIDTH))
            .child(picker)
            .into_any_element(),
    )
}

/// The optional plan a mission is started *from* (§5, M9).
///
/// **Paste is the one source this build offers.** A project file and a KB document are the other
/// two the design names, and both need the file's bytes read back before there is anything to
/// save — a round trip this dialog has no state machine for. Pasting the markdown asks nothing of
/// the host, so it is what is here; the other two are a card, not a hedge.
///
/// Folded away until asked for: most missions start without one, and a second text area open on
/// every mission is a dialog that reads as work.
fn plan_seed(
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

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(hint_row(
            "new-mission-plan-hint",
            "Plan",
            "Optional. A mission started from a plan opens in refining rather than requirements.",
            div()
                .flex()
                .flex_none()
                .justify_end()
                .w(px(CONTROL_WIDTH))
                .child(check_box(
                    "new-mission-plan-toggle",
                    form.plan_open,
                    cx.listener(|this, _, _, cx| this.toggle_new_mission_plan(cx)),
                ))
                .into_any_element(),
        ))
        .when(form.plan_open, |row| {
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
