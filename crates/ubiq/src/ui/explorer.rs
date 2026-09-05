//! The file tree panel.
//!
//! Tree and list are the same set the host has named, arranged twice — the same two arrangements
//! the file picker draws, through the same chrome in `ui::kit::files`. What is different is what
//! a row can carry: git colour, a leading mark, a badge, a loading or truncated note, and a
//! right-click menu. The picker ticks and confirms; this panel opens and decorates.

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, DragMoveEvent, Focusable,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, MouseDownEvent, ParentElement,
    Render, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, point,
    prelude::FluentBuilder, px,
};
use gpui_component::IconName;
use gpui_component::InteractiveElementExt;
use gpui_component::input::Input;

use crate::app::{AppState, MIN_QUERY};
use crate::state::{ExplorerKey, ExplorerView, Follow, GitStatus, MenuId, Row};
use crate::theme;
use crate::ui::eid;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{
    ContextItem, badge, context_menu, disclosure, elided, elided_with, file_row, filter_bar,
    icon_button, kind_icon, mono, panel, panel_header, twisty, view_switch,
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

/// The row under the pointer during a drag. It carries the path alone: where a drop would put it
/// is the folder's answer, not the drag's.
///
/// Its own payload type, so the external-file drop the centre panel already takes
/// (`on_drop::<ExternalPaths>`) is untouched by any of this.
#[derive(Clone)]
struct DraggedPath(String);

/// What follows the pointer during a drag. The row stays where it is and a label travels, the same
/// shape the task board's drag has.
struct Ghost(SharedString);

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(theme::surface_raised())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::accent())
            .text_size(px(12.))
            .text_color(theme::text())
            .child(self.0.clone())
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
        ExplorerDelete,
        ExplorerDismiss,
        ExplorerFocusTree,
        ExplorerFocusFilter
    ]
);

/// The key that removes the row the keyboard is on.
///
/// Backspace on macOS and Delete elsewhere, which is what each platform's own file manager uses.
/// Both are bound on every platform all the same: a keyboard is not always the one the operating
/// system came with, and the one that is idiomatic here is never wrong on the other.
const DELETE_KEYS: [&str; 4] = if cfg!(target_os = "macos") {
    ["backspace", "shift-backspace", "delete", "shift-delete"]
} else {
    ["delete", "shift-delete", "backspace", "shift-backspace"]
};

/// The keys the panel answers to, and where each is live.
///
/// **This is the default for every tree in Ubiq**, and the rule behind it is one sentence: the
/// filter above a tree and the tree itself are two focuses, and a key means what the focus it is
/// aimed at says it means. So the field keeps every key a field keeps — Backspace first of all,
/// which must never reach a file — and the tree's own keys are bound at the panel, where they are
/// only reachable once the tree holds the keyboard.
///
/// Three keys cross the boundary, because a field with a list under it is one control to the hand:
/// Down and Tab step from the field onto the tree, Escape clears the query from either, and Enter
/// opens what the query landed on without asking the user to leave the field first. Shift+Tab and
/// Tab step back off the tree onto the field.
pub fn key_bindings() -> Vec<KeyBinding> {
    fn panel<A: gpui::Action>(key: &str, action: A) -> KeyBinding {
        KeyBinding::new(key, action, Some(CONTEXT))
    }
    // The field's own, bound at the field's depth for the reason `ui::file_picker::key_bindings`
    // gives: the component library binds at the deepest node, so a binding on the panel alone
    // would never be reached while the caret is in the field.
    fn field<A: gpui::Action>(key: &str, action: A) -> KeyBinding {
        KeyBinding::new(key, action, Some(FIELD_CONTEXT))
    }

    let mut binds = vec![
        panel("up", ExplorerUp),
        panel("down", ExplorerDown),
        panel("left", ExplorerOut),
        panel("right", ExplorerInto),
        panel("enter", ExplorerEnter),
        panel("shift-enter", ExplorerShiftEnter),
        panel("escape", ExplorerDismiss),
        panel("tab", ExplorerFocusFilter),
        panel("shift-tab", ExplorerFocusFilter),
        field("down", ExplorerFocusTree),
        field("tab", ExplorerFocusTree),
        field("enter", ExplorerEnter),
        field("escape", ExplorerDismiss),
    ];
    // Delete is the panel's alone. There is no version of it at the field: Backspace is how a
    // query is corrected, and a tree that removed a file because the field had run out of letters
    // to delete would be indefensible.
    binds.extend(DELETE_KEYS.map(|key| panel(key, ExplorerDelete)));
    binds
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
    // Which folder a drop would land in, which is the only answer the user gets before letting go.
    let drop_onto = explorer.drop_onto.clone();

    // Whether the tree itself holds the keyboard, which is what deepens the cursor bar.
    let on_tree = app.explorer_focus.is_focused(window);
    // Whether what is typed is long enough to be searching for anything. Read off the field
    // rather than off the applied filter, so the text stops being faint on the keystroke that
    // clears the floor rather than when the walk behind it lands.
    let searching = app.file_filter.read(cx).value().trim().chars().count() >= MIN_QUERY;
    let drawn = explorer.drawn_rows(&app.workbench.file_filter);
    // The project's own row is drawn whatever the filter says, so "nothing matches" is what a
    // filter leaving nothing *under* it looks like.
    let filtered_out =
        !app.workbench.file_filter.trim().is_empty() && drawn.iter().all(|row| row.path.is_empty());
    let rows: Vec<AnyElement> = drawn
        .iter()
        .map(|row| {
            // The tree scales with the project's font size, the same knob as the editor and the
            // terminal, so a zoom dresses the whole project's workspace at once. The tree is the
            // densest surface, so it sits a half point under the editor's floor.
            let font = app.ui_font_size_or_default(cx) - 0.5;
            let lit = drop_onto.as_deref() == Some(row.path.as_str());
            line(row, tree, selected.as_deref(), font, lit, on_tree, cx)
        })
        .collect();

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
        .on_action(cx.listener(|this, _: &ExplorerDelete, window, cx| {
            answer(this, ExplorerKey::Delete, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerFocusTree, window, cx| {
            this.focus_explorer_tree(window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExplorerFocusFilter, window, cx| {
            this.focus_explorer_filter(window, cx)
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
                        // because that is where those two are asked for with no row in mind.
                        let at = event.position();
                        this.open_explorer_menu(None, (f32::from(at.x), f32::from(at.y)), cx);
                    }),
                ))
                .child(follow_button(explorer.follow, cx))
                .child(icon_button(
                    "explorer-collapse",
                    IconName::ChevronsUpDown,
                    false,
                    cx.listener(|this, _, _, cx| this.collapse_explorer(cx)),
                )),
        ))
        .children(bookmarks_section(app, cx))
        .child(filter_bar(
            // Under the three-character floor nothing is being searched for yet, and the query
            // says so by drawing faint rather than by a message the user has to read.
            Input::new(&app.file_filter)
                .appearance(false)
                .text_color(match searching {
                    true => theme::text(),
                    false => theme::text_muted(),
                }),
            div().flex().flex_none().items_center().gap_1().child(
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
                // The tree is a focus of its own, separate from the filter above it. Every key the
                // panel binds is live only from here.
                .track_focus(&app.explorer_focus)
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
                // The panel itself is the project's root: a row dragged onto empty space moves to
                // the top level. A folder row claims the drop before this sees it.
                .when(drop_onto.as_deref() == Some(""), |this| {
                    this.bg(theme::accent_soft())
                })
                .on_drag_move(
                    cx.listener(|this, event: &DragMoveEvent<DraggedPath>, _, cx| {
                        if event.bounds.contains(&event.event.position) {
                            this.drag_path_over(Some(String::new()), cx);
                        }
                    }),
                )
                .on_drop(cx.listener(|this, dragged: &DraggedPath, _, cx| {
                    this.drop_path_on(dragged.0.clone(), String::new(), cx);
                }))
                .children(filtered_out.then(|| empty_panel("Nothing matches")))
                .children(rows),
        );

    if menu_open && let Some(menu) = menu {
        let epoch = menu.epoch;
        let items: Vec<ContextItem> = menu
            .entries()
            .into_iter()
            .map(|entry| {
                if entry.is_separator() {
                    return ContextItem::separator();
                }
                let item = ContextItem::new(entry.label());
                match entry.enabled {
                    true => item,
                    false => item.disabled(),
                }
            })
            .collect();
        body = body.child(context_menu(
            "explorer-menu",
            point(px(menu.x), px(menu.y)),
            items,
            crate::ui::indexed(&cx.entity(), |this, index, window, cx| {
                this.pick_explorer_action(index, window, cx);
            }),
            crate::ui::handler(&cx.entity(), move |this, _, cx| {
                this.dismiss_explorer_menu(epoch, cx)
            }),
        ));
    }

    body.into_any_element()
}

fn line(
    row: &Row,
    tree: bool,
    selected: Option<&str>,
    font_size: f32,
    lit: bool,
    focused: bool,
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
        focused,
        font_size,
    );

    // A folder is a drop target: it says so by lighting up, and it claims the drop before the
    // panel behind it — which is the project's root — can take it.
    if row.is_dir && readable {
        let folder = row.path.clone();
        let dropped = row.path.clone();
        line = line
            .when(lit, |this| this.bg(theme::accent_soft()))
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DraggedPath>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        cx.stop_propagation();
                        this.drag_path_over(Some(folder.clone()), cx);
                    }
                }),
            )
            .on_drop(cx.listener(move |this, dragged: &DraggedPath, _, cx| {
                cx.stop_propagation();
                this.drop_path_on(dragged.0.clone(), dropped.clone(), cx);
            }));
    }

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

    // The project itself is not draggable: there is nowhere above it to drop it.
    if row.path.is_empty() {
        return line
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.click_explorer_row(String::new(), false, window, cx);
            }))
            .into_any_element();
    }

    let ghost: SharedString = row.name.clone().into();
    line = line.on_drag(DraggedPath(path.clone()), move |_, _, _, cx: &mut App| {
        let ghost = ghost.clone();
        cx.new(|_| Ghost(ghost))
    });

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

/// The follow button: a small square that says whether the tree tracks the active editor.
///
/// Three states in one control, cycled by clicking it — off, revealed once, and locked — because
/// the middle one is a verb and the last one is a mode, and a user asking for the first rarely
/// wants the second. Empty, half-lit, lit.
fn follow_button(state: Follow, cx: &mut Context<AppState>) -> impl IntoElement {
    let edge = match state {
        Follow::Off => theme::text_faint(),
        _ => theme::accent(),
    };
    let mut mark = div().size(px(10.)).border_1().border_color(edge);
    mark = match state {
        Follow::Off => mark,
        Follow::Once => mark.bg(theme::accent_soft()),
        Follow::Locked => mark.bg(theme::accent()),
    };
    div()
        .id("explorer-follow")
        .size(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(state.label()).build(window, cx)
        })
        .child(mark)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_explorer_follow(cx)))
}

/// The project's written-down places, above the filter and outside the tree's focus so neither
/// the tree's keymap nor its cursor knows this is here.
///
/// Plain rows, not a second tree: there is no twisty, no indent and no key handling — a bookmark
/// is a destination with a name, and the only thing to do with one is go there.
fn bookmarks_section(app: &AppState, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let marks: Vec<(usize, String, crate::state::Destination, bool)> = app
        .bookmarks(cx)
        .iter()
        .enumerate()
        .map(|(ix, mark)| (ix, mark.name.clone(), mark.dest.clone(), mark.adrift))
        .collect();
    if marks.is_empty() {
        return None;
    }
    let open = app.workbench.bookmarks_open;
    let count = marks.len().to_string();
    let font = app.ui_font_size_or_default(cx) - 0.5;

    let rows: Vec<AnyElement> = marks
        .into_iter()
        .map(|(ix, name, dest, adrift)| {
            let colour = if adrift {
                // A bookmark that lost its line says so; it is not quietly repaired.
                theme::warning()
            } else {
                theme::text_muted()
            };
            div()
                .id(("bookmark", ix))
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(2.))
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(elided(("bookmark-name", ix), name, colour, font))
                .when(adrift, |this| this.child(badge("adrift", theme::warning())))
                .on_click(cx.listener(move |this, _, _, cx| this.navigate(dest.clone(), cx)))
                .into_any_element()
        })
        .collect();

    Some(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(disclosure(
                "explorer-bookmarks",
                "Bookmarks",
                badge(&count, theme::text_faint()),
                open,
                cx.listener(|this, _, _, cx| this.toggle_bookmarks_section(cx)),
            ))
            .when(open, |this| {
                this.child(
                    div()
                        .id("explorer-bookmark-rows")
                        .flex()
                        .flex_col()
                        .flex_none()
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .children(rows),
                )
            })
            .into_any_element(),
    )
}
