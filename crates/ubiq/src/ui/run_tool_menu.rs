//! The titlebar's run picker: a filter field over every tool this project can run — the ones the
//! user wrote and the targets the host found in the project's runner files.
//!
//! Painted over the window for the reason [`super::new_project_menu`] is: the titlebar does not
//! name `AppState` beyond what it already reads. It is the **anchored list** shape, though, not a
//! menu: the field is the first thing in it and holds the keyboard, so the key context and the
//! handlers go on the field's wrapper — the navigator's device, for the same reason.
//!
//! The order is `state::run_picker::rows`'s: favourites, recents, the user's own tools, then the
//! discovered targets under a rule. Each row leads with its runner's icon, and ends with a star
//! and the controls its run state allows — Play for a tool with no live pane, Stop and Restart for
//! one that is running. Clicking the name brings a tool's pane forward, or runs the tool when it
//! has none. **A tool with a live pane is never offered a second run.**

use gpui::Focusable as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, anchored, deferred, div, point,
    px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use ubiq_proto::tools::ToolOrigin;

use crate::app::AppState;
use crate::ext::runner;
use crate::state::run_picker::{PickerRow, Section};
use crate::theme;
use crate::ui::kit::menu::MENU_LAYER;
use crate::ui::kit::{UbiqIcon, elided, file_row, filter_bar, row_font, section_label, status_dot};

/// The context the picker is answered in, and the one the component library gives the field
/// inside it.
const CONTEXT: &str = "RunPicker";
const FIELD_CONTEXT: &str = "RunPicker > Input";

/// The panel's width. Wide enough for a recipe name and the five controls a running row ends in.
const WIDTH: f32 = 360.;

/// The tallest the list grows before it scrolls.
const LIST_MAX: f32 = 360.;

gpui::actions!(
    ubiq_run_picker,
    [RunPickerUp, RunPickerDown, RunPickerEnter, RunPickerDismiss]
);

/// The keys the picker answers to, bound twice each: for the wrapper, and for the field inside it,
/// whose own `up`, `down`, `enter` and `escape` sit deeper and would otherwise win. See
/// [`crate::ui::navigator::key_bindings`].
pub fn key_bindings() -> Vec<KeyBinding> {
    fn both<A: gpui::Action + Clone>(key: &str, action: A) -> [KeyBinding; 2] {
        [
            KeyBinding::new(key, action.clone(), Some(CONTEXT)),
            KeyBinding::new(key, action, Some(FIELD_CONTEXT)),
        ]
    }

    [
        both("up", RunPickerUp),
        both("down", RunPickerDown),
        both("enter", RunPickerEnter),
        both("escape", RunPickerDismiss),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Draw the open picker, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.run_tool_menu else {
        return div().into_any_element();
    };
    let rows = app.run_picker_rows(cx);
    let cursor = app
        .workbench
        .run_tool_cursor
        .min(rows.len().saturating_sub(1));
    let focused = app
        .run_tool_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let field = div()
        .key_context(CONTEXT)
        .on_action(cx.listener(|this, _: &RunPickerUp, _, cx| this.move_run_picker(false, cx)))
        .on_action(cx.listener(|this, _: &RunPickerDown, _, cx| this.move_run_picker(true, cx)))
        .on_action(cx.listener(|this, _: &RunPickerEnter, _, cx| this.press_run_picker(cx)))
        .on_action(cx.listener(|this, _: &RunPickerDismiss, _, cx| this.dismiss_run_tool_menu(cx)))
        .child(filter_bar(
            gpui_component::input::Input::new(&app.run_tool_input).appearance(false),
            div(),
            focused,
        ));

    let mut children: Vec<AnyElement> = Vec::new();
    let mut section: Option<Section> = None;
    for (position, row) in rows.iter().enumerate() {
        if section != Some(row.section) {
            if let Some(label) = row.section.label() {
                children.push(
                    div()
                        .px_2()
                        .pt_1p5()
                        .pb_0p5()
                        .flex_none()
                        .child(section_label(label))
                        .into_any_element(),
                );
            } else if row.section == Section::Discovered && section.is_some() {
                // The rule between what the user wrote and what the host found.
                children.push(
                    div()
                        .my_1()
                        .h(px(1.))
                        .flex_none()
                        .bg(theme::border())
                        .into_any_element(),
                );
            }
            section = Some(row.section);
        }
        children.push(line(position, row, position == cursor, app, cx));
    }
    if children.is_empty() {
        let note = match (
            app.workbench.tools.is_empty(),
            app.workbench.run_tool_filter.is_empty(),
        ) {
            (true, _) => "No tools for this project",
            (false, _) => "No tool matches",
        };
        children.push(
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .text_color(theme::text_faint())
                .child(note)
                .into_any_element(),
        );
    }

    let panel = div()
        .id("run-tool-menu")
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        // Painted above whatever raised it, so a click inside must not reach the titlebar under.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_run_tool_menu(cx)))
        .child(field)
        .child(
            div()
                .id("run-tool-list")
                .flex()
                .flex_col()
                .max_h(px(LIST_MAX))
                .overflow_y_scroll()
                .children(children),
        );

    deferred(
        anchored()
            .position(point(px(at.0), px(at.1)))
            .snap_to_window_with_margin(px(8.))
            .child(panel),
    )
    .priority(MENU_LAYER)
    .into_any_element()
}

/// One row: the runner's mark, the name, and what can be done to it.
fn line(
    position: usize,
    row: &PickerRow,
    on_cursor: bool,
    app: &AppState,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let index = row.index;
    let listed = &app.workbench.tools[index];
    let name = listed.tool.name.clone();
    let favourite = app.tool_is_favorite(index, cx);
    let pane = app.tool_pane(index, cx);
    let running = pane.is_some_and(|(_, running)| running);

    let (mark, kind): (Icon, SharedString) = match &listed.origin {
        ToolOrigin::Defined => (UbiqIcon::RunnerUbiq.into(), "Your tool".into()),
        ToolOrigin::Discovered { runner: id } => (
            runner::icon_of(id),
            runner::kind_of(id).map_or_else(|| id.clone().into(), |kind| kind.label.into()),
        ),
    };

    // The name is the click target for "take me to it", not the whole row: the controls at the far
    // end are siblings of it, so none of them has to stop a click reaching the row.
    let target = div()
        .id(("run-tool-name", position))
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .id(("run-tool-kind", position))
                .flex_none()
                .child(mark.with_size(Size::XSmall).text_color(theme::text_muted()))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(kind.clone()).build(window, cx)
                }),
        )
        .child(elided(
            ("run-tool-label", position),
            name,
            theme::text(),
            theme::font(theme::Family::Chrome, theme::Role::Body),
        ))
        .on_click(cx.listener(move |this, _, _, cx| this.activate_tool(index, cx)));

    let star = control(
        ("run-tool-star", position),
        Glyph::Icon(if favourite {
            IconName::StarFill
        } else {
            IconName::Star
        }),
        if favourite { "Unstar" } else { "Star" },
        favourite,
        cx.listener(move |this, _, _, cx| this.toggle_tool_favorite(index, cx)),
    );

    let mut line = file_row(
        ("run-tool-row", position),
        0,
        false,
        on_cursor,
        true,
        row_font(),
    )
    .child(target)
    .when(running, |this| {
        // The state the row is in, by colour from the status group rather than by wording alone.
        this.child(status_dot(theme::success(), theme::success_soft()))
    })
    .child(star);

    line = match pane {
        Some((_, true)) => line
            .child(control(
                ("run-tool-stop", position),
                Glyph::Stop,
                "Stop",
                false,
                cx.listener(move |this, _, _, cx| this.stop_tool(index, cx)),
            ))
            .child(control(
                ("run-tool-restart", position),
                Glyph::Icon(IconName::RotateCw),
                "Restart",
                false,
                cx.listener(move |this, _, _, cx| this.restart_tool(index, cx)),
            )),
        // Held but stopped: running it again replaces the spent pane.
        Some((_, false)) => line.child(control(
            ("run-tool-play", position),
            Glyph::Icon(IconName::Play),
            "Run again",
            false,
            cx.listener(move |this, _, _, cx| this.play_tool(index, cx)),
        )),
        None => line.child(control(
            ("run-tool-play", position),
            Glyph::Icon(IconName::Play),
            "Run",
            false,
            cx.listener(move |this, _, _, cx| this.play_tool(index, cx)),
        )),
    };
    line.into_any_element()
}

/// What a control draws. Stop is a drawn square: `gpui-component` has no stop glyph.
enum Glyph {
    Icon(IconName),
    Stop,
}

/// A row-height control at the end of a row: flush, no margin, a tooltip because the glyph alone
/// does not say what it does.
fn control(
    id: (&'static str, usize),
    glyph: Glyph,
    tip: &'static str,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let colour = if active {
        theme::accent()
    } else {
        theme::text_muted()
    };
    div()
        .id(id)
        .w(px(24.))
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(active, |this| this.bg(theme::accent_soft()))
        .hover(|this| this.bg(theme::hover()))
        .child(match glyph {
            Glyph::Icon(name) => Icon::new(name)
                .with_size(Size::XSmall)
                .text_color(colour)
                .into_any_element(),
            Glyph::Stop => div().size(px(8.)).bg(colour).into_any_element(),
        })
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
        .on_click(on_click)
        .into_any_element()
}
