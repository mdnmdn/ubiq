//! The clone modal: where a project comes from when it is not a folder the user already has.
//!
//! One modal rather than a wizard, because every field is answerable at any time: a connection's
//! listing and a pasted URL sit side by side, and picking either is what says which one the clone
//! uses. The two are never both live — [`CloneMode`] says which — and the half that is not chosen
//! is drawn dimmed rather than hidden, so switching back is a click and not a rediscovery.
//!
//! **The footer is the whole of the progress report.** Once a clone is running the confirm button
//! becomes Cancel and the stage reads beside it; nothing else in the body changes, so a cancelled
//! clone is a corrected field away from being started again.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, Focusable, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};
use ubiq_proto::repos::RemoteRepo;

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::clone::{CloneMode, CloneState, check_url, clone_error_note, stage_note};
use crate::theme;
use crate::ui::kit::{
    Picker, check_box, elided, field, ghost_button, label_block, modal_note, modal_sized,
    primary_button, section_label,
};
use crate::ui::{handler, indexed};

/// Wider than the kit's default and given a height to fill: the repository list is the body's
/// point, and a list inside a hugging modal would collapse to whatever it happened to hold.
const CLONE_WIDTH: f32 = 560.0;
const CLONE_HEIGHT: f32 = 620.0;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(clone) = app.workbench.clone_project.as_ref() else {
        return div().into_any_element();
    };
    let view = cx.entity();

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_3()
        .pt_3()
        .children(clone.error.as_ref().map(|error| {
            div()
                .px_2()
                .py_2()
                .flex_none()
                .bg(theme::danger_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::danger())
                .text_size(px(11.5))
                .text_color(theme::text())
                .child(clone_error_note(error))
        }))
        .children(connector_picker(app, clone, cx))
        .child(repo_list(app, clone, window, cx))
        .child(url_field(app, clone, window, cx))
        .child(branch_picker(app, clone, window, cx))
        .child(destination(app, clone, window, cx))
        .child(options(clone, cx))
        .into_any_element();

    modal_sized(
        "clone-modal",
        theme::accent(),
        CLONE_WIDTH,
        Some(CLONE_HEIGHT),
        "Clone a project",
        body,
        footer(clone, cx),
        handler(&view, |this, _, cx| this.close_clone(cx)),
        window,
    )
}

/// Whose repositories are being listed.
///
/// Absent entirely when there are no connections: a picker over nothing is a control that says
/// "you cannot do this" where the paste field beneath it says otherwise.
fn connector_picker(
    app: &AppState,
    clone: &CloneState,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let connections = &app.workbench.settings.host.connections;
    if connections.is_empty() {
        return None;
    }
    let view = cx.entity();
    let labels: Vec<String> = connections
        .iter()
        .map(|c| format!("{} \u{b7} {}", c.provider.label(), c.label))
        .collect();
    let at = connections
        .iter()
        .position(|c| Some(c.id) == clone.connection);
    let ids: Vec<_> = connections.iter().map(|c| c.id).collect();
    let picked = at.map_or_else(|| "Choose an identity".to_string(), |ix| labels[ix].clone());

    let mut picker = Picker::new("clone-connection", picked)
        .items(labels)
        .open(app.workbench.open_menu == Some(MenuId::CloneConnection))
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::CloneConnection, cx)
        }))
        .on_pick(indexed(&view, move |this, index, _, cx| {
            if let Some(id) = ids.get(index).copied() {
                this.pick_clone_connection(id, cx);
            }
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)));
    if let Some(at) = at {
        picker = picker.selected(at);
    }

    Some(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .child(label_block("Identity", "Whose repositories to list."))
            .child(picker)
            .into_any_element(),
    )
}

/// The listing: a filter, and the rows it leaves.
///
/// The filter runs in memory — see [`CloneState::visible`] — and only reaches the provider when it
/// runs out of local answers over a listing the provider truncated. That is why an empty result
/// says which of the two it is rather than a flat "no matches".
fn repo_list(
    app: &AppState,
    clone: &CloneState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = app
        .clone_filter_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let found = clone.visible();
    let dim = clone.mode == CloneMode::Url;

    let rows: Vec<AnyElement> = match found.is_empty() {
        true => vec![
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(12.5))
                .text_color(theme::text_faint())
                .child(match (clone.repos_query.is_some(), clone.truncated) {
                    (true, _) => "Listing\u{2026}",
                    (false, true) => "Nothing here yet \u{2014} still searching the provider.",
                    (false, false) => "No repositories match.",
                })
                .into_any_element(),
        ],
        false => found.iter().map(|repo| row(clone, repo, cx)).collect(),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_2()
        .when(dim, |this| this.opacity(0.5))
        .child(section_label("Repositories"))
        .child(
            field(theme::border(), focused)
                .h(px(28.))
                .px_2()
                .flex_none()
                .gap_2()
                .child(
                    Icon::new(IconName::Search)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.5))
                        .child(Input::new(&app.clone_filter_input).appearance(false)),
                ),
        )
        .child(
            div()
                .id("clone-repos")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .bg(theme::surface())
                .children(rows),
        )
        .into_any_element()
}

/// One repository. Its full name, its visibility, and whatever the provider said about it.
fn row(clone: &CloneState, repo: &RemoteRepo, cx: &mut Context<AppState>) -> AnyElement {
    let selected = clone
        .repo
        .as_ref()
        .is_some_and(|picked| picked.id == repo.id)
        && clone.mode == CloneMode::Connection;
    let picked = repo.clone();

    div()
        .id(ElementId::Name(format!("clone-repo-{}", repo.id).into()))
        .h(px(30.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .when(selected, |this| this.bg(theme::accent_soft()))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.5))
                .text_color(match selected {
                    true => theme::text(),
                    false => theme::text_muted(),
                })
                .truncate()
                .child(repo.full_name.clone()),
        )
        .children(repo.private.then(|| {
            Icon::new(IconName::Eye)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint())
        }))
        .children(repo.description.as_ref().map(|note| {
            elided(
                ElementId::Name(format!("clone-repo-note-{}", repo.id).into()),
                note.clone(),
                theme::text_faint(),
                200.,
            )
        }))
        .on_click(
            cx.listener(move |this, _, window, cx| {
                this.pick_clone_repo(picked.clone(), window, cx)
            }),
        )
        .into_any_element()
}

/// The other half: a URL, validated as it is typed.
///
/// An ssh remote parses and is refused with a sentence — see `state::clone::check_url` — because
/// "not a repository" would be a lie about a URL that plainly is one.
fn url_field(
    app: &AppState,
    clone: &CloneState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = app
        .clone_url_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let checked = check_url(&clone.url);
    let edge = match &checked {
        Some(Err(_)) => theme::danger(),
        _ => theme::border(),
    };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .when(clone.mode == CloneMode::Connection, |this| {
            this.opacity(0.5)
        })
        .child(label_block(
            "Or a URL",
            "A public repository, cloned anonymously.",
        ))
        .child(
            field(edge, focused)
                .h(px(28.))
                .px_2()
                .child(Input::new(&app.clone_url_input).appearance(false)),
        )
        .children(match checked {
            Some(Err(note)) => Some(
                div()
                    .text_size(px(11.5))
                    .text_color(theme::danger())
                    .child(note),
            ),
            _ => None,
        })
        .into_any_element()
}

/// Which branch the clone checks out.
///
/// While the listing is still arriving this holds the repository's own default and nothing else,
/// so the control says what would happen rather than sitting empty.
fn branch_picker(
    app: &AppState,
    clone: &CloneState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let search_focused = app
        .picker_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let typed = app.picker_search.read(cx).value().to_string();
    let needle = typed.trim().to_lowercase();
    let shown: Vec<String> = clone
        .branches
        .iter()
        .filter(|name| crate::state::navigator::subsequence(&needle, name))
        .cloned()
        .collect();
    let at = clone
        .branch
        .as_ref()
        .and_then(|picked| shown.iter().position(|name| name == picked));
    let label = clone
        .branch
        .clone()
        .unwrap_or_else(|| "The default branch".to_string());
    let pickable = shown.clone();

    let mut picker = Picker::new("clone-branch", label)
        .items(shown)
        .search(&app.picker_search, search_focused)
        .open(app.workbench.open_menu == Some(MenuId::CloneBranch))
        .on_toggle(handler(&view, |this, window, cx| {
            this.open_picker_menu(MenuId::CloneBranch, window, cx)
        }))
        .on_pick(indexed(&view, move |this, index, _, cx| {
            if let Some(branch) = pickable.get(index).cloned() {
                this.pick_clone_branch(branch, cx);
            }
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)));
    if let Some(at) = at {
        picker = picker.selected(at);
    }

    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(label_block("Branch", "What the clone checks out."))
        .child(picker)
        .into_any_element()
}

/// Where it lands, and what the folder is called there.
fn destination(
    app: &AppState,
    clone: &CloneState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = app
        .clone_name_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let (path, colour) = match clone.parent.trim().is_empty() {
        true => ("The host's own default".to_string(), theme::text_faint()),
        false => (clone.parent.clone(), theme::text_muted()),
    };

    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(label_block(
            "Destination",
            "The folder the clone is created inside, and what it is called there.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(elided("clone-parent", path, colour, 300.))
                .child(ghost_button(
                    "clone-parent-choose",
                    None,
                    "Choose\u{2026}",
                    cx.listener(|this, _, _, cx| this.choose_clone_folder(cx)),
                )),
        )
        .child(
            field(theme::border(), focused)
                .h(px(28.))
                .px_2()
                .child(Input::new(&app.clone_name_input).appearance(false)),
        )
        .into_any_element()
}

/// The two switches. Ticking ephemeral moves the destination and turns shallow on; both stay the
/// user's to change afterwards, which is why they are boxes and not a mode.
fn options(clone: &CloneState, cx: &mut Context<AppState>) -> AnyElement {
    let tick = |id: &'static str,
                on: bool,
                label: &'static str,
                note: &'static str,
                cx: &mut Context<AppState>,
                toggle: fn(&mut AppState, &mut Context<AppState>)| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(check_box(
                id,
                on,
                cx.listener(move |this, _, _, cx| toggle(this, cx)),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(theme::text())
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::text_faint())
                            .child(note),
                    ),
            )
    };

    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(tick(
            "clone-ephemeral",
            clone.ephemeral,
            "Throwaway",
            "Lands under the ephemeral folder, and closing it discards the clone.",
            cx,
            |this, cx| this.toggle_clone_ephemeral(cx),
        ))
        .child(tick(
            "clone-shallow",
            clone.shallow,
            "Shallow",
            "The tip only. Cheap, and wrong the moment anyone wants history.",
            cx,
            |this, cx| this.toggle_clone_shallow(cx),
        ))
        .into_any_element()
}

/// Cancel and confirm, or — while a clone is running — the stage and a Cancel that stops it.
fn footer(clone: &CloneState, cx: &mut Context<AppState>) -> AnyElement {
    if let Some(stage) = &clone.stage {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_between()
            .gap_2()
            .child(modal_note(&stage_note(stage)))
            .child(ghost_button(
                "clone-stop",
                None,
                "Cancel",
                cx.listener(|this, _, _, cx| this.cancel_clone(cx)),
            ))
            .into_any_element();
    }

    let ready = clone.source().is_some() && !clone.name.trim().is_empty();
    let confirm = primary_button(
        "clone-start",
        None,
        "Clone",
        cx.listener(|this, _, _, cx| this.start_clone(cx)),
    );

    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "clone-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_clone(cx)),
        ))
        .child(match ready {
            true => confirm,
            false => confirm.opacity(0.5),
        })
        .into_any_element()
}
