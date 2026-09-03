//! The editable half of the task panel: everything on a task the user can change.
//!
//! It is not an area of its own — it fills the panel that already has one, which is why there is no
//! row for it in the workbench's table and no size constant of its own. It is a second file because
//! [`super::detail`] is the *report* and this is the *controls*, the same split
//! `ui/chat/{transcript,composer}` draws.
//!
//! Three rules shape all of it.
//!
//! **One field at a time.** The panel reports a task first, and a panel where every field is a text
//! box has stopped reporting. A field opens on a click and closes on a commit, exactly as a picker
//! row expands into a rename and back.
//!
//! **Every edit asks and waits.** Nothing here writes to the projection: the field sends, the host
//! answers, and the panel goes on reporting the task the host last confirmed. So a refusal leaves
//! nothing to unwind, and a value equal to the one the host already holds sends nothing at all.
//!
//! **A status is not here.** A column is a stage, and a card only ever changes column by being
//! moved — a picker for it would be a second way to do the one thing the drag is for.

use gpui::{
    AnyElement, App, Context, Focusable, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::{Input, Textarea};
use gpui_component::text::TextView;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::{Priority, Shape, TaskRecord};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::board::Field;
use crate::theme;
use crate::ui::kit::{
    Picker, PickerStyle, choice_pill, field, ghost_button, icon_button, mono, primary_button,
    section_label, toggle_pill,
};
use crate::ui::{eid, eid2, handler, indexed};

/// The title: what the task is called, and the one field that cannot be emptied.
pub fn title(
    app: &AppState,
    task: &TaskRecord,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let editing = app
        .board(cx)
        .is_some_and(|board| board.is_editing(Field::Title));

    if !editing {
        return div()
            .id("board-title")
            .text_size(px(17.))
            .text_color(theme::text())
            .cursor_text()
            .hover(|this| this.text_color(theme::accent()))
            .child(SharedString::from(task.title.clone()))
            .on_click(
                cx.listener(|this, _, window, cx| this.begin_task_edit(Field::Title, window, cx)),
            )
            .into_any_element();
    }

    let focused = app
        .task_title_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            field(theme::accent(), focused)
                .flex_1()
                .min_w(px(0.))
                .px_2()
                .py_1()
                .text_size(px(15.))
                .child(Input::new(&app.task_title_input).appearance(false)),
        )
        .child(icon_button(
            "board-title-save",
            IconName::Check,
            false,
            cx.listener(|this, _, window, cx| this.commit_task_title(window, cx)),
        ))
        .child(icon_button(
            "board-title-cancel",
            IconName::Close,
            false,
            cx.listener(|this, _, window, cx| this.cancel_task_edit(window, cx)),
        ))
        .into_any_element()
}

/// Priority and shape: three fixed values each, so the row of pills *is* both the report and the
/// control and there is no edit mode to enter.
pub fn pills(task: &TaskRecord, cx: &mut Context<AppState>) -> AnyElement {
    let priorities: Vec<AnyElement> = Priority::all()
        .into_iter()
        .map(|priority| {
            choice_pill(
                eid("board-priority", priority.label().unwrap_or("normal")),
                priority.label().unwrap_or("normal"),
                task.priority == priority,
                cx.listener(move |this, _, _, cx| this.set_task_priority(priority, cx)),
            )
            .into_any_element()
        })
        .collect();

    let shapes: Vec<AnyElement> = Shape::all()
        .into_iter()
        .map(|shape| {
            choice_pill(
                eid("board-shape", shape.label()),
                shape.label().to_lowercase(),
                task.shape == shape,
                cx.listener(move |this, _, _, cx| this.set_task_shape(shape, cx)),
            )
            .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(section_label("Priority"))
                .children(priorities),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(section_label("Shape"))
                .children(shapes),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child(task.shape.note()),
        )
        .into_any_element()
}

/// The session the work belongs to. A picker rather than a pill row: the list is as long as the
/// project has sessions, and it grows.
pub fn session(app: &AppState, task: &TaskRecord, cx: &mut Context<AppState>) -> AnyElement {
    let Some(work) = app.work(cx) else {
        return div().into_any_element();
    };
    let view = cx.entity().clone();

    // The unassigned row first, so "nobody has started this" is a choice rather than an absence
    // the user has to work out how to get back to.
    let mut items: Vec<SharedString> = vec!["no session yet".into()];
    let ids: Vec<_> = work.sessions.iter().map(|s| s.id).collect();
    items.extend(
        work.sessions
            .iter()
            .map(|s| SharedString::from(s.name.clone())),
    );

    let selected = task
        .session
        .and_then(|id| ids.iter().position(|s| *s == id))
        .map(|ix| ix + 1)
        .unwrap_or(0);
    let label = items[selected].clone();

    Picker::new("board-session-pick", label)
        .items(items.iter().map(|item| item.to_string()))
        .selected(selected)
        .style(PickerStyle::Chip)
        .open(app.workbench.open_menu == Some(MenuId::TaskSession))
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::TaskSession, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        .on_pick(indexed(&view, move |this, index, _, cx| {
            // Index zero is the unassigned row, so everything below it is one off the session list.
            let session = index.checked_sub(1).and_then(|ix| ids.get(ix).copied());
            this.set_task_session(session, cx);
        }))
        .into_any_element()
}

/// The description, as markdown, with one control that swaps it for the source.
///
/// Rendered by default, because a description is read far more often than it is written. The
/// component library's defaults are what this wants and each override would be wrong: it is
/// selectable already, and it must **not** scroll, because the panel around it does and a scroller
/// inside a scroller would let a long description hide the sub-tasks under it.
pub fn description(
    app: &AppState,
    task: &TaskRecord,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(board) = app.board(cx) else {
        return div().into_any_element();
    };
    let editing = board.is_editing(Field::Description);
    let preview = board.preview;

    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(section_label("Description"))
        .child(div().flex_1().min_w(px(0.)))
        .children(editing.then(|| {
            toggle_pill(
                "board-desc-preview",
                "preview",
                theme::accent(),
                preview,
                cx.listener(|this, _, _, cx| this.toggle_description_preview(cx)),
            )
        }))
        .children((!editing).then(|| {
            ghost_button(
                "board-desc-write",
                Some(IconName::Replace),
                "Write",
                cx.listener(|this, _, window, cx| {
                    this.begin_task_edit(Field::Description, window, cx)
                }),
            )
        }));

    let body = if editing && !preview {
        let focused = app
            .task_description_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        field(theme::accent(), focused)
            .id("board-desc-editor")
            .flex_col()
            .items_stretch()
            .px_2()
            .py_1()
            .cursor_text()
            .child(
                Textarea::new(&app.task_description_input)
                    .appearance(false)
                    .bordered(false)
                    .w_full()
                    .text_size(px(13.)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                let input = this.task_description_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }))
            .into_any_element()
    } else {
        // While the preview is up the draft is what it renders, not the record: the point of the
        // control is to see what has just been typed.
        let source = if editing {
            board.form.description.clone()
        } else {
            task.description.clone()
        };
        rendered(task, source, cx)
    };

    let mut root = div().flex().flex_col().gap_1p5().child(header).child(body);

    if editing {
        root = root.child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(primary_button(
                    "board-desc-save",
                    None,
                    "Save",
                    cx.listener(|this, _, _, cx| this.commit_task_description(cx)),
                ))
                .child(ghost_button(
                    "board-desc-cancel",
                    None,
                    "Cancel",
                    cx.listener(|this, _, window, cx| this.cancel_task_edit(window, cx)),
                )),
        );
    }

    root.into_any_element()
}

/// The markdown itself, or a line saying there is none.
///
/// An absent description is drawn as absent rather than by dropping the section, on the rule the
/// status bar and the explorer's git marks both follow: a fact nobody has filled in is still worth
/// showing a space for.
fn rendered(task: &TaskRecord, source: String, cx: &mut Context<AppState>) -> AnyElement {
    if source.trim().is_empty() {
        return div()
            .id("board-desc-empty")
            .text_size(px(12.5))
            .text_color(theme::text_faint())
            .cursor_text()
            .child("No description yet.")
            .on_click(cx.listener(|this, _, window, cx| {
                this.begin_task_edit(Field::Description, window, cx)
            }))
            .into_any_element();
    }

    div()
        .id("board-desc-read")
        .cursor_text()
        .child(
            TextView::markdown(eid("task-md", task.id), source)
                .text_size(px(theme::EDITOR_FONT_SIZE)),
        )
        .on_click(
            cx.listener(|this, _, window, cx| this.begin_task_edit(Field::Description, window, cx)),
        )
        .into_any_element()
}

/// The two controls a sub-task row grows: rename it, or drop it.
///
/// No question before the ×. The two-click question is for what cannot be retyped — a dirty buffer,
/// a forgotten project — and a sub-task's title is one line.
pub fn step_controls(
    app: &AppState,
    task: &TaskRecord,
    step: ubiq_proto::ids::StepId,
    cx: &mut Context<AppState>,
) -> AnyElement {
    if app
        .board(cx)
        .is_some_and(|board| board.is_editing(Field::Step(step)))
    {
        return div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(icon_button(
                eid2("board-step-save", task.id, step),
                IconName::Check,
                false,
                cx.listener(|this, _, window, cx| this.commit_step_title(window, cx)),
            ))
            .child(icon_button(
                eid2("board-step-cancel", task.id, step),
                IconName::Close,
                false,
                cx.listener(|this, _, window, cx| this.cancel_task_edit(window, cx)),
            ))
            .into_any_element();
    }

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(icon_button(
            eid2("board-step-edit", task.id, step),
            IconName::Replace,
            false,
            cx.listener(move |this, _, window, cx| {
                this.begin_task_edit(Field::Step(step), window, cx)
            }),
        ))
        .child(icon_button(
            eid2("board-step-drop", task.id, step),
            IconName::Close,
            false,
            cx.listener(move |this, _, _, cx| this.remove_task_step(step, cx)),
        ))
        .into_any_element()
}

/// The field a sub-task is renamed in, shown in place of its title.
pub fn step_field(app: &AppState, window: &Window, cx: &App) -> AnyElement {
    let focused = app
        .step_title_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    field(theme::accent(), focused)
        .flex_1()
        .min_w(px(0.))
        .px_2()
        .py(px(1.))
        .text_size(px(13.))
        .child(Input::new(&app.step_title_input).appearance(false))
        .into_any_element()
}

/// The field at the foot of the list. Enter adds and keeps the focus, so several sub-tasks can be
/// typed in a row without reaching for the mouse.
pub fn new_step(app: &AppState, window: &Window, cx: &App) -> AnyElement {
    let focused = app
        .new_step_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    field(theme::border(), focused)
        .h(px(26.))
        .px_2()
        .flex_none()
        .gap_2()
        .child(
            Icon::new(IconName::Plus)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.5))
                .child(Input::new(&app.new_step_input).appearance(false)),
        )
        .into_any_element()
}

/// Delete, and the question it asks first.
pub fn delete(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let asking = app.board(cx).is_some_and(|board| board.confirm_delete);

    if !asking {
        return ghost_button(
            "board-task-delete",
            Some(IconName::Delete),
            "Delete",
            cx.listener(|this, _, _, cx| this.delete_task(cx)),
        )
        .into_any_element();
    }

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(mono("Delete this task?", theme::danger()).text_size(px(11.5)))
        .child(ghost_button(
            "board-task-delete-yes",
            None,
            "Delete",
            cx.listener(|this, _, _, cx| this.delete_task(cx)),
        ))
        .child(ghost_button(
            "board-task-delete-no",
            None,
            "Keep",
            cx.listener(|this, _, _, cx| this.withdraw_task_delete(cx)),
        ))
        .into_any_element()
}

/// The last thing the host refused to do to the work, said where the user is looking.
///
/// Not in the project picker beside a catalogue failure: a task that would not move is not a fact
/// about the catalogue, and the panel is the thing the user was touching when it happened.
pub fn refusal(app: &AppState) -> Option<AnyElement> {
    let message = app.workbench.work_error.clone()?;
    Some(
        div()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_1p5()
            .bg(theme::danger_soft())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::danger())
            .child(
                Icon::new(IconName::TriangleAlert)
                    .with_size(Size::XSmall)
                    .text_color(theme::danger()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(12.))
                    .text_color(theme::text())
                    .child(SharedString::from(message)),
            )
            .into_any_element(),
    )
}
