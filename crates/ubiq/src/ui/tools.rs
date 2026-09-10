//! The runnable-tools panels: the machine-wide list in application settings, and one project's
//! own list in its settings. One panel twice, because the rows and the editor are the same and
//! only the list behind them differs — which list is the [`ToolEditScope`] the editor carries.
//!
//! [`ToolEditScope`]: crate::state::settings::ToolEditScope

use gpui::{
    AnyElement, Context, ElementId, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use gpui_component::IconName;
use gpui_component::input::{Input, Textarea};

use ubiq_proto::tools::{TOOL_PLATFORMS, ToolDef, parse_env};

use crate::app::AppState;
use crate::state::settings::{ToolEditScope, ToolEditor};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    check_box, choice_pill, field, ghost_button, heading, icon_button, label_block, mono,
    primary_button,
};

/// The platform multiple choice: the value a row stores, and what its pill says.
const PLATFORM_LABELS: [(&str, &str); 3] = [
    ("macos", "macOS"),
    ("windows", "Windows"),
    ("linux", "Linux"),
];

/// One tools list and its editor, for the machine-wide rows or one project's own.
///
/// The fields draw unfocused, the way the Isolation section's grant field does: the editor
/// commits on Save rather than on blur, so focus styling would promise immediacy the form
/// does not have.
pub fn panel(app: &AppState, cx: &mut Context<AppState>, scope: ToolEditScope) -> AnyElement {
    let prefix = match &scope {
        ToolEditScope::System => "system-tools",
        ToolEditScope::Project(_) => "project-tools",
    };
    let mut rows = vec![match &scope {
        ToolEditScope::System => heading(
            "Tools",
            "Named commands this machine runs in a terminal pane — builds, watchers, shells \
             with flags. A project page adds rows of its own, below these.",
        ),
        ToolEditScope::Project(_) => heading(
            "Tools",
            "Named commands run in this project's folder — on top of the machine-wide rows in \
             Settings › Tools.",
        ),
    }];

    let tools = app.tool_list(&scope, cx);
    if tools.is_empty() {
        rows.push(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child("No tools yet. Add a build, a watcher, or a shell with flags — the `+` menu offers them beside the shells.")
                .into_any_element(),
        );
    }
    for tool in &tools {
        rows.push(tool_row(prefix, &scope, tool, cx));
    }

    match app.workbench.settings.tool_editor.clone() {
        Some(editor) if editor.scope == scope => {
            rows.push(editor_form(app, cx, prefix, &editor));
        }
        _ => {
            let scope_add = scope.clone();
            rows.push(
                ghost_button(
                    ElementId::Name(format!("{prefix}-add").into()),
                    Some(IconName::Plus),
                    "Add tool",
                    cx.listener(move |this, _, window, cx| {
                        this.begin_add_tool(scope_add.clone(), window, cx);
                    }),
                )
                .into_any_element(),
            );
        }
    }

    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .children(rows)
        .into_any_element()
}

/// One saved row: the name the tab will say, what it runs, and its edit and remove controls.
fn tool_row(
    prefix: &str,
    scope: &ToolEditScope,
    tool: &ToolDef,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = tool.id;
    let scope_edit = scope.clone();
    let scope_drop = scope.clone();
    let mut command = tool.command.clone();
    if !tool.args.trim().is_empty() {
        command.push(' ');
        command.push_str(tool.args.trim());
    }
    let platforms = match tool.platforms.as_slice() {
        [] => "all platforms".to_string(),
        picked => picked.join(", "),
    };
    let summary = format!(
        "{} · {} · {}",
        command,
        platforms,
        if tool.wait_on_exit {
            "waits on exit"
        } else {
            "closes on exit"
        }
    );
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    mono(tool.name.clone(), theme::text())
                        .text_size(theme::font(Family::Chrome, Role::Body)),
                )
                .child(
                    mono(summary, theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(ghost_button(
                    ElementId::Name(format!("{prefix}-edit-{id}").into()),
                    None,
                    "Edit",
                    cx.listener(move |this, _, window, cx| {
                        this.begin_edit_tool(scope_edit.clone(), id, window, cx);
                    }),
                ))
                .child(icon_button(
                    ElementId::Name(format!("{prefix}-remove-{id}").into()),
                    IconName::Close,
                    false,
                    cx.listener(move |this, _, _, cx| {
                        this.remove_tool(scope_drop.clone(), id, cx);
                    }),
                )),
        )
        .into_any_element()
}

/// The add-or-edit form: four textboxes and two choices, committed on Save.
fn editor_form(
    app: &AppState,
    cx: &mut Context<AppState>,
    prefix: &str,
    editor: &ToolEditor,
) -> AnyElement {
    let editing = editor.id.is_some();
    let (_, ignored) = parse_env(&app.tool_env_input.read(cx).value());

    let mut form = div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(label_block(
            "Name",
            "What the tab says. A number joins it when two panes run the same tool.",
        ))
        .child(
            field(theme::border(), false)
                .h(px(28.))
                .w(px(320.))
                .px_2()
                .child(
                    Input::new(&app.tool_name_input)
                        .appearance(false)
                        .text_size(theme::font(Family::Chrome, Role::Body)),
                ),
        )
        .child(label_block(
            "Command",
            "The program — a bare name, a path, or a program with its first arguments.",
        ))
        .child(
            field(theme::border(), false)
                .h(px(28.))
                .w(px(320.))
                .px_2()
                .child(
                    Input::new(&app.tool_command_input)
                        .appearance(false)
                        .text_size(theme::font(Family::Chrome, Role::Body)),
                ),
        )
        .child(label_block(
            "Parameters",
            "Split the way a shell splits them: quotes group words, backslashes stay.",
        ))
        .child(
            field(theme::border(), false)
                .h(px(28.))
                .w(px(320.))
                .px_2()
                .child(
                    Input::new(&app.tool_args_input)
                        .appearance(false)
                        .text_size(theme::font(Family::Chrome, Role::Body)),
                ),
        )
        .child(label_block(
            "Environment",
            "Extra variables for the run, one KEY=VALUE per line.",
        ))
        .child(
            field(theme::border(), false).p_2().w(px(320.)).child(
                Textarea::new(&app.tool_env_input)
                    .appearance(false)
                    .bordered(false)
                    .w_full()
                    .text_size(theme::font(Family::Chrome, Role::Body)),
            ),
        );
    if !ignored.is_empty() {
        form = form.child(
            mono(
                SharedString::from(format!(
                    "{} {} ignored — want KEY=VALUE.",
                    ignored.len(),
                    if ignored.len() == 1 { "line" } else { "lines" }
                )),
                theme::warning(),
            )
            .text_size(theme::font(Family::Chrome, Role::Meta)),
        );
    }

    let platforms: Vec<_> = TOOL_PLATFORMS
        .iter()
        .map(|platform| {
            let label = PLATFORM_LABELS
                .iter()
                .find(|(value, _)| *value == *platform)
                .map(|(_, label)| *label)
                .unwrap_or(*platform);
            let active = editor
                .platforms
                .iter()
                .any(|picked| picked.as_str() == *platform);
            let platform = platform.to_string();
            choice_pill(
                ElementId::Name(format!("{prefix}-platform-{platform}").into()),
                label,
                active,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_tool_platform(&platform, cx);
                }),
            )
        })
        .collect();
    form = form
        .child(label_block(
            "Platforms",
            "Where this runs. Nothing picked runs everywhere.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_wrap()
                .children(platforms),
        );

    let wait = editor.wait_on_exit;
    form = form.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(check_box(
                ElementId::Name(format!("{prefix}-wait").into()),
                wait,
                cx.listener(|this, _, _, cx| {
                    this.toggle_tool_wait(cx);
                }),
            ))
            .child(label_block(
                "Wait on exit",
                "Keep the pane open after the command ends, so its output stays readable.",
            )),
    );

    form.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(primary_button(
                ElementId::Name(format!("{prefix}-save").into()),
                None,
                if editing { "Save tool" } else { "Add tool" },
                cx.listener(move |this, _, _, cx| {
                    this.save_tool_editor(cx);
                }),
            ))
            .child(ghost_button(
                ElementId::Name(format!("{prefix}-cancel").into()),
                None,
                "Cancel",
                cx.listener(|this, _, _, cx| {
                    this.cancel_tool_editor(cx);
                }),
            )),
    )
    .into_any_element()
}
