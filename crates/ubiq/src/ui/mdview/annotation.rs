//! **The annotation layer over a row:** what the page's margins carry while the host has pushed
//! decor ([`MdView::set_decor`]) and, for the far-margin stack, while the view is annotating
//! ([`MdView::set_annotating`]).
//!
//! - Every row with threads gets a **count marker** in the far margin (annotating only).
//! - The **selected** row gets the action stack under it: `⚑` (mark menu), `✓`/`↺` (resolve or
//!   reopen), `+` (new thread), `●` (highlight menu), `✎` (block editor, only while editable), and
//!   an accent bar down its left edge.
//! - Every highlighted row gets a colour dot in the left margin, in every mode (`highlight.rs`).
//!
//! **Nothing here may change a row's measured height, or sit on its text.** `ListState` keeps a
//! sum-tree of measured heights that the scrollbar, the minimap and the split's scroll link all
//! read, so every decorator is `absolute` — out of flow by construction — and the stack lives in
//! the page's far margin, which `Metrics` floors at `theme::MD_ACTION_MARGIN` while annotating.
//!
//! **The view never mutates decor.** Every control is an intent ([`MdViewEvent`]); the host acts
//! on its own state and pushes fresh decor back. The open menu is the view's — there is one.
//!
//! Ported from the markdown-viewer spike's `ui/annotation.rs` `row` / `actions` (mission T-298).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, anchored, deferred, div, px,
};
use ubiq_proto::plan::{AnnotationMark, HighlightColour};

use super::blocks::Metrics;
use super::highlight;
use super::view::MdView;
use crate::state::document::RowDecor;
use crate::theme::{self, Family, Role};
use crate::ui::kit::menu::MENU_LAYER;

/// Every mark, in the order the `⚑` menu lists them.
pub const MARKS: [AnnotationMark; 3] = [
    AnnotationMark::Agent,
    AnnotationMark::Todo,
    AnnotationMark::Question,
];

/// The word a mark is listed under.
pub fn mark_label(mark: AnnotationMark) -> &'static str {
    match mark {
        AnnotationMark::Agent => "Agent",
        AnnotationMark::Todo => "Todo",
        AnnotationMark::Question => "Question",
    }
}

/// The one row menu that may be open, and on which row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowMenu {
    Mark(usize),
    Highlight(usize),
}

/// The thread-count marker's facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    /// Open plus resolved threads on the row.
    pub threads: usize,
    /// Every thread on the row is resolved.
    pub all_resolved: bool,
    /// An open thread on the row carries the Agent mark.
    pub for_agent: bool,
    /// The focused thread is on this row.
    pub focused: bool,
}

/// What a row draws in its margins — a pure reading of the decor, the selection and the mode, so
/// a test asserts the layer without pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowActions {
    pub marker: Option<Marker>,
    /// The accent bar down the selected row's left edge.
    pub selected_bar: bool,
    pub add_thread: bool,
    pub mark: bool,
    /// `Some(true)` draws `✓` (resolve), `Some(false)` draws `↺` (reopen).
    pub resolve: Option<bool>,
    pub highlight_menu: bool,
    pub edit: bool,
    /// The left-margin dot's colour, in every mode.
    pub dot: Option<HighlightColour>,
}

impl RowActions {
    /// What row `decor` (if any) draws with the row `selected` or not, in `annotating` mode, with
    /// the block editor `editable`.
    pub fn of(decor: Option<&RowDecor>, selected: bool, annotating: bool, editable: bool) -> Self {
        let threads = decor.map_or(0, |d| d.open + d.resolved);
        let stack = selected && annotating;
        RowActions {
            marker: (annotating && threads > 0).then(|| {
                let d = decor.expect("a row with threads has decor");
                Marker {
                    threads,
                    all_resolved: d.open == 0,
                    for_agent: d.open > 0 && d.marks.contains(&AnnotationMark::Agent),
                    focused: d.focused,
                }
            }),
            selected_bar: stack,
            add_thread: stack,
            mark: stack,
            resolve: decor.filter(|_| stack && threads > 0).map(|d| d.open > 0),
            highlight_menu: stack,
            edit: stack && editable,
            dot: decor.and_then(|d| d.highlight),
        }
    }

    fn any_far(&self) -> bool {
        self.marker.is_some()
            || self.add_thread
            || self.mark
            || self.resolve.is_some()
            || self.highlight_menu
            || self.edit
    }
}

/// The absolute layers row `ix` carries over its rendered block: the selection bar, the highlight
/// dot and the far-margin stack. Empty when the row has none.
pub fn overlays(
    view: &WeakEntity<MdView>,
    this: &MdView,
    ix: usize,
    m: &Metrics,
) -> Vec<AnyElement> {
    let decor = this.decor_at(ix);
    let actions = this.row_actions(ix);
    let mut out = Vec::new();
    // Another writer just changed this row (`D208`): a tint under everything else, fading out.
    let flash = this.flash_alpha(ix);
    if flash > 0.0 {
        out.push(
            div()
                .absolute()
                .inset_0()
                .bg(theme::fade(theme::agent_edit_flash(), flash))
                .into_any_element(),
        );
    }
    if actions.selected_bar {
        out.push(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(px(theme::ACCENT_EDGE))
                .bg(theme::accent())
                .into_any_element(),
        );
    }
    if let Some(colour) = actions.dot {
        out.push(highlight::dot(view, ix, m, colour));
    }
    if actions.any_far() {
        out.push(stack(view, this, decor, &actions, ix, m));
    }
    out
}

/// The far-margin column: marker, `⚑`, `✓`/`↺`, `+`, `●`, `✎`, centred on the row. Every control
/// stops propagation, or the row's own click would select (and a quick second click edit).
fn stack(
    view: &WeakEntity<MdView>,
    this: &MdView,
    decor: Option<&RowDecor>,
    actions: &RowActions,
    ix: usize,
    m: &Metrics,
) -> AnyElement {
    let menu = this.open_menu();
    let marker = actions.marker.map(|marker| {
        let reveal = view.clone();
        let tip = format!(
            "{} thread{}",
            marker.threads,
            if marker.threads == 1 { "" } else { "s" }
        );
        action(("md-threads", ix), marker.threads.to_string(), tip)
            .debug_selector(move || format!("md-threads-{ix}"))
            .bg(theme::accent_soft())
            .text_color(theme::accent())
            .when(marker.for_agent, |this| {
                this.bg(theme::agent_controlled_soft())
                    .text_color(theme::agent_controlled())
            })
            .when(marker.all_resolved, |this| {
                this.bg(theme::surface_raised())
                    .text_color(theme::text_faint())
            })
            .when(marker.focused, |this| {
                this.bg(theme::accent()).text_color(theme::on_accent())
            })
            .on_click(move |_: &ClickEvent, _window, cx: &mut App| {
                cx.stop_propagation();
                reveal
                    .update(cx, |this, cx| this.threads_clicked(ix, cx))
                    .ok();
            })
            .into_any_element()
    });

    let mark = actions.mark.then(|| {
        let open = menu == Some(RowMenu::Mark(ix));
        let on = decor.map(|d| d.marks.clone()).unwrap_or_default();
        div()
            .relative()
            .child(menu_chip(
                view,
                ("md-mark-open", ix),
                "⚑",
                "Mark",
                RowMenu::Mark(ix),
                open,
            ))
            .children(open.then(|| mark_menu(view, ix, &on)))
            .into_any_element()
    });

    let resolve = actions.resolve.map(|resolving| {
        let act = view.clone();
        let (glyph, tip) = if resolving {
            ("✓", "Resolve")
        } else {
            ("↺", "Reopen")
        };
        action(("md-resolve", ix), glyph, tip)
            .when(resolving, |this| this.text_color(theme::success()))
            .on_click(move |_: &ClickEvent, _window, cx: &mut App| {
                cx.stop_propagation();
                act.update(cx, |this, cx| this.request_resolve(ix, cx)).ok();
            })
            .into_any_element()
    });

    let add = actions.add_thread.then(|| {
        let start = view.clone();
        action(("md-new-thread", ix), "+", "New thread")
            .debug_selector(move || format!("md-new-thread-{ix}"))
            .on_click(move |_: &ClickEvent, _window, cx: &mut App| {
                cx.stop_propagation();
                start.update(cx, |this, cx| this.add_thread(ix, cx)).ok();
            })
            .into_any_element()
    });

    let paint = actions.highlight_menu.then(|| {
        highlight::chip(
            view,
            ix,
            decor.and_then(|d| d.highlight),
            menu == Some(RowMenu::Highlight(ix)),
        )
    });

    let edit = actions.edit.then(|| {
        let open = view.clone();
        action(("md-edit-open", ix), "✎", "Edit block")
            .on_click(move |_: &ClickEvent, window, cx: &mut App| {
                cx.stop_propagation();
                open.update(cx, |this, cx| super::blockedit::open(this, ix, window, cx))
                    .ok();
            })
            .into_any_element()
    });

    div()
        .absolute()
        .top_0()
        .right_0()
        .h_full()
        .w(px(m.margin))
        .flex()
        .flex_col()
        .justify_center()
        .items_center()
        .gap_1()
        .children(marker)
        .children(mark)
        .children(resolve)
        .children(add)
        .children(paint)
        .children(edit)
        .into_any_element()
}

/// One square control of the stack: a glyph, named in a tooltip.
pub(super) fn action(
    id: impl Into<ElementId>,
    glyph: impl Into<SharedString>,
    tip: impl Into<SharedString>,
) -> Stateful<Div> {
    let tip: SharedString = tip.into();
    div()
        .id(id)
        .size(px(theme::MD_ACTION_SIZE))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .text_size(theme::font(Family::Chrome, Role::Micro))
        .text_color(theme::text_muted())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()).text_color(theme::text()))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
        .child(glyph.into())
}

/// A control that opens `which`, and — while it is open — closes it.
///
/// The close is a **capture-phase** mouse-down, not a click: the menu's own `on_mouse_down_out` is
/// capture-phase too and would close it on the press, letting the release's click open it again.
/// The chip paints before the deferred menu, so its capture listener runs first, and stopping
/// propagation there keeps both the menu's close and the chip's own click from firing.
pub(super) fn menu_chip(
    view: &WeakEntity<MdView>,
    id: impl Into<ElementId>,
    glyph: &'static str,
    tip: &'static str,
    which: RowMenu,
    open: bool,
) -> Stateful<Div> {
    let chip = action(id, glyph, tip);
    let view = view.clone();
    if open {
        chip.bg(theme::accent_soft())
            .capture_any_mouse_down(move |_, _window, cx: &mut App| {
                cx.stop_propagation();
                view.update(cx, |this, cx| this.close_menu(cx)).ok();
            })
    } else {
        chip.on_click(move |_: &ClickEvent, _window, cx: &mut App| {
            cx.stop_propagation();
            view.update(cx, |this, cx| this.open_row_menu(which, cx))
                .ok();
        })
    }
}

/// A row menu's panel: `kit::popover`'s chrome (raised, accent left edge), but hung by its
/// top-right corner from the chip so it opens leftward over the page rather than off the margin.
/// It occludes and stops its own presses; a press anywhere else closes it.
pub(super) fn menu_panel(
    view: &WeakEntity<MdView>,
    id: impl Into<ElementId>,
    entries: impl IntoIterator<Item = AnyElement>,
) -> AnyElement {
    let close = view.clone();
    let panel = div()
        .id(id)
        .occlude()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(move |_, _, cx: &mut App| {
            close.update(cx, |this, cx| this.close_menu(cx)).ok();
        })
        .flex()
        .flex_col()
        .p_1()
        .mr_1()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        .children(entries);
    deferred(
        anchored()
            .anchor(Anchor::TopRight)
            .snap_to_window_with_margin(px(8.))
            .child(panel),
    )
    .priority(MENU_LAYER)
    .into_any_element()
}

/// One entry of a row menu: an optional leading swatch, then its label; lit when `on`.
pub(super) fn menu_entry(
    id: impl Into<ElementId>,
    label: &'static str,
    on: bool,
    leading: Option<AnyElement>,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .cursor_pointer()
        .map(|this| {
            if on {
                this.bg(theme::accent_soft()).text_color(theme::text())
            } else {
                this.text_color(theme::text_muted())
                    .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
            }
        })
        .children(leading)
        .child(label)
        .on_click(on_click)
}

/// The `⚑` menu: one toggle per mark, lit for those the row's open threads carry. A pick is
/// [`MdView::request_mark`].
fn mark_menu(view: &WeakEntity<MdView>, ix: usize, on: &[AnnotationMark]) -> AnyElement {
    let entries = MARKS.into_iter().enumerate().map(|(i, mark)| {
        let pick = view.clone();
        menu_entry(
            ("md-mark-item", i),
            mark_label(mark),
            on.contains(&mark),
            None,
            move |_: &ClickEvent, _window, cx: &mut App| {
                cx.stop_propagation();
                pick.update(cx, |this, cx| this.request_mark(ix, mark, cx))
                    .ok();
            },
        )
        .into_any_element()
    });
    menu_panel(view, ("md-mark-menu", ix), entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decor(open: usize, resolved: usize) -> RowDecor {
        RowDecor {
            row: 0,
            open,
            resolved,
            marks: Vec::new(),
            highlight: None,
            focused: false,
        }
    }

    #[test]
    fn the_stack_is_the_selected_rows_while_annotating() {
        let d = decor(1, 0);
        let selected = RowActions::of(Some(&d), true, true, true);
        assert!(selected.add_thread && selected.mark && selected.highlight_menu && selected.edit);
        assert_eq!(selected.resolve, Some(true));
        assert!(selected.selected_bar);

        let other = RowActions::of(Some(&d), false, true, true);
        assert_eq!(other.marker.map(|m| m.threads), Some(1));
        assert!(!other.add_thread && !other.mark && other.resolve.is_none());

        let preview = RowActions::of(Some(&d), true, false, true);
        assert_eq!(preview, RowActions::default());
    }

    #[test]
    fn a_fully_resolved_row_reopens_and_reads_faint() {
        let d = decor(0, 2);
        let actions = RowActions::of(Some(&d), true, true, false);
        assert_eq!(actions.resolve, Some(false));
        assert!(
            actions
                .marker
                .is_some_and(|m| m.all_resolved && m.threads == 2)
        );
        assert!(!actions.edit);
        // No threads: nothing to resolve, but a new thread is still on offer.
        let bare = RowActions::of(None, true, true, false);
        assert_eq!(bare.resolve, None);
        assert!(bare.marker.is_none() && bare.add_thread);
    }

    #[test]
    fn the_dot_draws_in_every_mode() {
        let mut d = decor(0, 0);
        d.highlight = Some(HighlightColour::Green);
        for annotating in [false, true] {
            let actions = RowActions::of(Some(&d), false, annotating, false);
            assert_eq!(actions.dot, Some(HighlightColour::Green));
            assert!(actions.marker.is_none());
        }
    }
}
