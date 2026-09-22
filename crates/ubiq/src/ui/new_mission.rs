//! The new-mission dialog: title, description, `require plan`, and an assistant picker filtered
//! to profiles carrying `mission_assistant` — `_docs/wip/planning-system.md`'s "new-mission
//! dialog and the assistant launch".
//!
//! Labelled with the project's own word for a mission ([`crate::app::AppState::mission_term`]), so
//! a team that renamed it to "epic" sees "New epic" both on the board's button and here.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, Focusable, IntoElement, ParentElement, Styled, Window, div, px,
};
use gpui_component::input::{Input, Textarea};

use crate::app::AppState;
use crate::state::Layer;
use crate::state::new_mission::{assistant_label, assistants};
use crate::theme;
use crate::ui::kit::menu::{Picker, PickerStyle};
use crate::ui::kit::{check_box, field, ghost_button, hint_row, label_hint, modal, primary_button};
use crate::ui::{handler, indexed};

const CONTROL_WIDTH: f32 = 230.;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.new_mission.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let term = app.mission_term(cx);
    // `profiles_in` rather than the global list alone: a profile scoped to this project is on
    // offer here too, not only its own global-list siblings — G331.
    let project = app.project(cx);
    let candidate_profiles = app.workbench.settings.profiles_in(project);
    let candidates = assistants(&candidate_profiles);

    let mut body = div().flex().flex_col().gap_3().pt_1();

    body = body.child(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_hint(
                "new-mission-title-hint",
                "Title",
                "What to call this mission.",
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
                "Description",
                "Carried in the assistant's opening briefing, and written onto the mission.",
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

    body = body.child(hint_row(
        "new-mission-require-plan-hint",
        "Require plan",
        "A reminder said in the assistant's opening briefing, not something the host enforces.",
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
                    "No profile is marked as a mission assistant yet. Tick \u{201c}Mission \
                     assistant\u{201d} on one in the profile editor to offer it here.",
                ),
        );
    } else {
        let ids: Vec<String> = candidates
            .iter()
            .map(|profile| profile.id.clone())
            .collect();
        let selected = form
            .assistant
            .as_ref()
            .and_then(|id| ids.iter().position(|it| it == id));
        let label = assistant_label(&form, &candidates, "Choose an assistant\u{2026}");

        let mut picker = Picker::new(ElementId::Name("new-mission-assistant".into()), label)
            .items(ids.clone())
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
            .on_pick(indexed(&view, move |this, index, _window, cx| {
                if let Some(id) = ids.get(index).cloned() {
                    this.pick_new_mission_assistant(id, cx);
                }
            }));

        body = body.child(hint_row(
            "new-mission-assistant-hint",
            "Assistant",
            "Filtered to profiles carrying the mission assistant flag.",
            div()
                .flex_none()
                .w(px(CONTROL_WIDTH))
                .child(picker)
                .into_any_element(),
        ));
    }

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
