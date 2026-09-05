//! The window's element tree.
//!
//! `kit/` holds the reusable primitives; every other module here draws one area of the workbench.
//! Nothing under `ui/` names the coordinator, a process, a path on disk or a file descriptor.
//!
//! Screen areas are free functions over the root view rather than views of their own, which keeps
//! one place — `AppState` — responsible for state and redraws.

pub mod agents;
pub mod board;
pub mod chat;
pub mod conversation;
pub mod dock;
pub mod editor;
pub mod empty;
pub mod explorer;
pub mod file_dialog;
pub mod file_picker;
pub mod file_tab_menu;
pub mod git;
pub mod kit;
pub mod logs;
pub mod navigator;
pub mod new_pane_menu;
pub mod orchestration;
pub mod project_menu;
pub mod rail;
pub mod ribbon;
pub mod search;
pub mod settings;
pub mod shell;
pub mod sink;
pub mod status_bar;
pub mod terminal;
pub mod titlebar;
pub mod viewer;
pub mod work;

use gpui::{App, ClickEvent, Context, ElementId, Entity, SharedString, Window};

use crate::app::AppState;

/// Adapt a method on the root view into the plain handler the kit expects.
///
/// The kit is view-agnostic on purpose, so it takes `Fn(&mut Window, &mut App)`; `cx.listener`
/// produces a click handler instead. This bridges the two by capturing the view.
pub fn handler(
    view: &Entity<AppState>,
    f: impl Fn(&mut AppState, &mut Window, &mut Context<AppState>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let view = view.clone();
    move |window, cx| {
        view.update(cx, |this, cx| f(this, window, cx));
    }
}

/// The same, for the kit's index-carrying callbacks: tab strips and menu rows.
pub fn indexed(
    view: &Entity<AppState>,
    f: impl Fn(&mut AppState, usize, &mut Window, &mut Context<AppState>) + 'static,
) -> impl Fn(usize, &mut Window, &mut App) + 'static {
    let view = view.clone();
    move |index, window, cx| {
        view.update(cx, |this, cx| f(this, index, window, cx));
    }
}

/// An element id for a row keyed by a ULID. A ULID is not a `u64`, so the tuple form the rest of
/// the window uses cannot carry one.
pub fn eid(prefix: &str, id: impl std::fmt::Display) -> ElementId {
    ElementId::Name(format!("{prefix}-{id}").into())
}

/// The same for a row two ids deep — a step, which is one id inside another.
pub fn eid2(prefix: &str, a: impl std::fmt::Display, b: impl std::fmt::Display) -> ElementId {
    ElementId::Name(format!("{prefix}-{a}-{b}").into())
}

/// Follow a link written inside a rendered document.
///
/// `base` is the document's own path, which a relative target is resolved against; `None` is the
/// project root, which is where a chat message and a task description are written from. A target
/// that names a place is navigated to, the web and mail are handed to the operating system, and
/// anything else is nothing at all — without this, a clicked `../src/app.rs` is handed to the
/// operating system as a URL.
pub fn on_link(
    app: Entity<AppState>,
    base: Option<SharedString>,
) -> impl Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync + 'static {
    move |target, _, _, cx| {
        let base = base.clone().unwrap_or_default();
        let dest = app
            .read(cx)
            .project(cx)
            .and_then(|project| crate::state::nav::resolve_relative(project, &base, target));
        match dest {
            Some(dest) => app.update(cx, |this, cx| this.navigate(dest, cx)),
            None => {
                let lower = target.to_ascii_lowercase();
                if ["http:", "https:", "mailto:"]
                    .iter()
                    .any(|scheme| lower.starts_with(scheme))
                {
                    cx.open_url(target);
                }
            }
        }
    }
}
