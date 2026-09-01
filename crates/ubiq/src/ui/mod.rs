//! The window's element tree.
//!
//! `kit/` holds the reusable primitives; every other module here draws one area of the workbench.
//! Nothing under `ui/` names the coordinator, a process, a path on disk or a file descriptor.
//!
//! Screen areas are free functions over the root view rather than views of their own, which keeps
//! one place — `AppState` — responsible for state and redraws.

pub mod agents;
pub mod chat;
pub mod editor;
pub mod empty;
pub mod explorer;
pub mod kit;
pub mod logs;
pub mod project_menu;
pub mod rail;
pub mod shell;
pub mod status_bar;
pub mod terminal;
pub mod titlebar;

use gpui::{App, Context, Entity, Window};

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
