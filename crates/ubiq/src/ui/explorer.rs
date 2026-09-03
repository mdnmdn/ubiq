//! The file tree panel.
//!
//! Tree and list are the same set the host has named, arranged twice — the same two arrangements
//! the file picker draws, through the same chrome in `ui::kit::files`. What is different is what
//! a row can carry: git colour, a leading mark, a badge, a loading or truncated note, and a
//! right-click menu. The picker ticks and confirms; this panel opens and decorates.

use gpui::{
    AnyElement, ClickEvent, Context, Focusable, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, ParentElement, Rgba, StatefulInteractiveElement, Styled, Window,
    div, point, px,
};
use gpui_component::IconName;
use gpui_component::InteractiveElementExt;
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::{ExplorerKey, ExplorerView, GitStatus, MenuId, Row};
use crate::theme;
use crate::ui::eid;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{
    ContextItem, badge, context_menu, elided, elided_with, file_row, filter_bar, icon_button,
    kind_icon, mono, panel, panel_header, twisty, view_switch,
};

/// The colour a row's name and dot take from its git state. Status is never shown by wording alone.
///
/// Nothing to say about a file is the muted default, which is also what every row looks like until
/// something reads a repository.
pub fn git_colour(status: Option<GitStatus>) -> Rgba {
    match status {
        None => theme::text_muted(),
        Some(GitStatus::Modified) => theme::warning(),
        Some(GitStatus::Untracked) => theme::success(),
        Some(GitStatus::Conflict) => theme::danger(),
        Some(GitStatus::Staged) => theme::info(),
        Some(GitStatus::Ignored) => theme::text_faint(),
    }
}

fn name_colour(status: Option<GitStatus>, readable: bool) -> Rgba {
    match status {
        None if readable => theme::text(),
        // Something the host will not open reads as unavailable rather than as unremarkable.
        None => theme::text_faint(),
        Some(GitStatus::Ignored) => theme::text_faint(),
        Some(other) => git_colour(Some(other)),
    }
}

fn icon_colour(status: Option<GitStatus>, readable: bool) -> Rgba {
    match status {
        Some(GitStatus::Ignored) | None if !readable => theme::text_faint(),
        Some(_) => git_colour(status),
        None => theme::text_faint(),
    }
}

/// The key context the panel is answered in, and the one the component library gives the field
/// inside it.
const CONTEXT: &str = "Explorer";
const FIELD_CONTEXT: &str = "Explorer > Input";

gpui::actions!(
    ubiq_explorer,
    [
        ExplorerUp,
        ExplorerDown,
        ExplorerOut,
        ExplorerInto,
        ExplorerEnter,
        ExplorerShiftEnter,
        ExplorerDismiss
    ]
);

/// The keys the panel answers to, bound twice each — once for the panel and once for the field
/// inside it, for the same reason the picker's are: the focus is in the filter, and the component
/// library's input binds the arrows for itself at the deepest node. See
/// `ui::file_picker::key_bindings`.
pub fn key_bindings() -> Vec<KeyBinding> {
    fn both<A: gpui::Action + Clone>(key: &str, action: A) -> [KeyBinding; 2] {
        [
            KeyBinding::new(key, action.clone(), Some(CONTEXT)),
            KeyBinding::new(key, action, Some(FIELD_CONTEXT)),
        ]
    }

    [
        both("up", ExplorerUp),
        both("down", ExplorerDown),
        both("left", ExplorerOut),
        both("right", ExplorerInto),
        both("enter", ExplorerEnter),
        both("shift-enter", ExplorerShiftEnter),
        both("escape", ExplorerDismiss),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn answer(
    this: &mut AppState,
    key: ExplorerKey,
    window: &mut gpui::Window,
    cx: &mut Context<AppState>,
) {
    if !this.press_explorer_key(key, window, cx) {
        cx.propagate();
    }
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    // With no project there is no folder to list. The panel stays, so the emptiness is explained
    // and the width the user dragged survives a project opening.
    if app.project(cx).is_none() {
        return panel()
            .border_r_1()
            .border_color(theme::border())
            .child(panel_header("Explorer", div()))
            .child(empty_panel("No project open"))
            .into_any_element();
    }

    // The tree belongs to the project; the filter belongs to the window, because there is one
    // field above N trees.
    let Some(explorer) = app.explorer(cx) else {
        return div().into_any_element();
    };
    let selected = explorer.selected.clone();
    let view = explorer.view;
    let tree = view == ExplorerView::Tree;
    let menu = explorer.menu.clone();
    let menu_open = app.workbench.open_menu == Some(MenuId::Explorer);

    let rows: Vec<AnyElement> = explorer
        .drawn_rows(&app.workbench.file_filter)
        .iter()
        .map(|row| {
            // The tree scales with the project's font size, the same knob as the editor and the
            // terminal, so a zoom dresses the whole project's workspace at once. The tree is the
            // densest surface, so it sits a half point under the editor's floor.
            let font = app.ui_font_size_or_default(cx) - 0.5;
            line(row, tree, selected.as_deref(), font, cx)
        })
        .collect();
    let filtered_out = rows.is_empty() && !app.workbench.file_filter.trim().is_empty();

    let mut body = panel()
        .id("explorer")
        .key_context(CONTEXT)
        .on_action(
            cx.listener(|this, _: &ExplorerUp, window, cx| {
                answer(this, ExplorerKey::Up, window, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &ExplorerDown, window, cx| {
            answer(this, ExplorerKey::Down, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerOut, window, cx| {
            answer(this, ExplorerKey::Left, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerInto, window, cx| {
            answer(this, ExplorerKey::Right, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerEnter, window, cx| {
            answer(this, ExplorerKey::Enter, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerShiftEnter, window, cx| {
            answer(this, ExplorerKey::ShiftEnter, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerDismiss, window, cx| {
            answer(this, ExplorerKey::Dismiss, window, cx)
        }))
        .border_r_1()
        .border_color(theme::border())
        .child(panel_header(
            "Explorer",
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(view_switch(
                    "explorer-tree",
                    "explorer-list",
                    tree,
                    cx.listener(|this, _, _, cx| this.set_explorer_view(ExplorerView::Tree, cx)),
                    cx.listener(|this, _, _, cx| this.set_explorer_view(ExplorerView::List, cx)),
                ))
                .child(icon_button(
                    "explorer-new",
                    IconName::Plus,
                    false,
                    cx.listener(|this, event: &ClickEvent, _, cx| {
                        // The plus is the empty-area menu: new file and new folder live there,
                        // even while those two still wait on the host.
                        let at = event.position();
                        this.open_explorer_menu(None, (f32::from(at.x), f32::from(at.y)), cx);
                    }),
                ))
                .child(icon_button(
                    "explorer-collapse",
                    IconName::ChevronsUpDown,
                    false,
                    cx.listener(|this, _, _, cx| this.collapse_explorer(cx)),
                )),
        ))
        .child(filter_bar(
            Input::new(&app.file_filter).appearance(false),
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(
                    mono(view.label(), theme::text_faint())
                        .text_size(px(10.5))
                        .flex_none()
                        .px_1()
                        .bg(theme::surface_raised()),
                )
                .child(
                    mono("\u{2318}P", theme::text_faint())
                        .text_size(px(10.5))
                        .px_1()
                        .bg(theme::surface_raised()),
                ),
            app.file_filter.read(cx).focus_handle(cx).is_focused(window),
        ))
        .child(
            div()
                .id("explorer-tree")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .track_scroll(&app.explorer_scroll)
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.open_explorer_menu(
                            None,
                            (f32::from(event.position.x), f32::from(event.position.y)),
                            cx,
                        );
                    }),
                )
                .children(filtered_out.then(|| empty_panel("Nothing matches")))
                .children(rows),
        );

    if menu_open && let Some(menu) = menu {
        let items: Vec<ContextItem> = menu
            .entries()
            .into_iter()
            .map(|entry| {
                let item = ContextItem::new(entry.label());
                match entry.ready() {
                    true => item,
                    false => item.disabled(),
                }
            })
            .collect();
        body = body.child(context_menu(
            "explorer-menu",
            point(px(menu.x), px(menu.y)),
            items,
            crate::ui::indexed(&cx.entity(), |this, index, _, cx| {
                this.pick_explorer_action(index, cx);
            }),
            crate::ui::handler(&cx.entity(), |this, _, cx| this.dismiss_explorer_menu(cx)),
        ));
    }

    body.into_any_element()
}

fn line(
    row: &Row,
    tree: bool,
    selected: Option<&str>,
    font_size: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let path = row.path.clone();
    let is_selected = selected == Some(row.path.as_str());
    let readable = row.readable;

    let mut line = file_row(
        eid("explorer-row", &row.path),
        row.depth,
        is_selected,
        row.on_cursor,
        font_size,
    );

    if tree && row.is_dir {
        let folder = row.path.clone();
        line = line.child(twisty(
            eid("explorer-twisty", &row.path),
            row.expanded,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_folder(folder.clone(), cx);
            }),
        ));
    }

    line = line.child(kind_icon(row.is_dir, icon_colour(row.git, readable)));

    line = line.child(elided_with(
        eid("explorer-name", &row.path),
        row.name.clone(),
        match row.path.is_empty() {
            true => row.name.clone(),
            false => row.path.clone(),
        },
        name_colour(row.git, readable),
        font_size,
    ));

    line = line.child(div().flex_1().min_w(px(0.)));

    if let Some(status) = row.git {
        line = line.child(badge(status.badge(), git_colour(row.git)));
    }

    if row.loading && row.expanded {
        line = line.child(
            mono("\u{2026}", theme::text_faint())
                .text_size(px(11.))
                .flex_none(),
        );
    }

    if row.truncated {
        line = line.child(
            mono("+", theme::text_faint())
                .text_size(px(11.))
                .flex_none(),
        );
    }

    if !row.trailing.is_empty() {
        line = line.child(
            div()
                .flex()
                .flex_none()
                .max_w(px(140.))
                .font_family(theme::MONO_FONT)
                .child(elided(
                    eid("explorer-trailing", &row.path),
                    row.trailing.clone(),
                    theme::text_faint(),
                    11.5,
                )),
        );
    }

    let menu_path = path.clone();
    line = line.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.open_explorer_menu(
                Some(menu_path.clone()),
                (f32::from(event.position.x), f32::from(event.position.y)),
                cx,
            );
        }),
    );

    // A row the host will not follow is drawn and does nothing: there is nothing behind it to
    // list or to open. The menu still opens, so the path can be copied.
    if !readable {
        return line.into_any_element();
    }

    let double_path = path.clone();
    line.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
        let mods = event.modifiers();
        let permanent = mods.shift || mods.platform;
        this.click_explorer_row(path.clone(), permanent, window, cx);
    }))
    .on_double_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        this.double_click_explorer_row(double_path.clone(), cx);
    }))
    .into_any_element()
}
