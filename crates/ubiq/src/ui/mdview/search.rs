//! Find in document: the navigation state, the per-row hit plumbing, and the find bar — a field,
//! the case and whole-word facets, prev/next and a close, floated over the page's top-right corner
//! (`Cmd+F` opens it, `Enter`/`Shift+Enter` step, `Esc` closes). One implementation for every
//! `MdView`: the markdown preview and the annotation surface are the same view.
//!
//! The engine owns the scan — [`ubiq_md::Document::find`] returns every hit as a root block index
//! plus a source byte range, non-overlapping and in source order — and owns **no navigation
//! state**. [`Find`] is that state: which hit is current, which options are on, and the query the
//! list was computed for. One per view (the `MdView` entity, T-301), not a thread-local as in the
//! spike: Ubiq has one view per markdown tab, and each keeps its own query.
//!
//! **The matches are recomputed from the document the render path is handed**, keyed on the block
//! count plus the first block's hash. Nothing hooks the reparse: a document that reparsed behind
//! the debounce has a different key by the next frame, and the match list is rebuilt before any row
//! draws with it.
//!
//! **A row's highlight travels out of band.** `blocks::row` opens a [`scope`] for the root block it
//! is about to build and `inline::runs` asks [`row_hits`] for it, so no signature between the two
//! had to grow a search parameter — the alternative was threading `Option<&Hits>` through every
//! block kind, the nested stack, and the table cells. The hand-off is a thread-local that lives
//! exactly as long as one row is being built, which is safe because rows build on the one thread
//! that draws and never nest.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    KeyBinding, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, actions, div, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use ubiq_md::{Document, FindOptions, Match};

use super::view::MdView;
use crate::theme::{self, Family, Role};

actions!(md_find, [MdFindOpen, MdFindNext, MdFindPrev, MdFindClose]);

/// The bar's key context, and the predicate its keys are bound under so they win inside the field.
const KEY_CONTEXT: &str = "MdFind";
const KEY_PREDICATE: &str = "MdFind > Input";

/// The find keys, for `app::install_key_bindings` to register after the library's own.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-f", MdFindOpen, Some("MdView")),
        KeyBinding::new("ctrl-f", MdFindOpen, Some("MdView")),
        KeyBinding::new("cmd-g", MdFindNext, Some("MdView")),
        KeyBinding::new("cmd-shift-g", MdFindPrev, Some("MdView")),
        KeyBinding::new("f3", MdFindNext, Some("MdView")),
        KeyBinding::new("shift-f3", MdFindPrev, Some("MdView")),
        KeyBinding::new("cmd-f", MdFindOpen, Some(KEY_PREDICATE)),
        KeyBinding::new("enter", MdFindNext, Some(KEY_PREDICATE)),
        KeyBinding::new("shift-enter", MdFindPrev, Some(KEY_PREDICATE)),
        KeyBinding::new("escape", MdFindClose, Some(KEY_PREDICATE)),
    ]
}

// ── The state ───────────────────────────────────────────────────────

/// What the cached match list was computed from. Any change here invalidates it, and the document
/// half of it — the block count and the first block's hash — is what makes a reparse behind the
/// debounce invalidate it without this module hearing about the reparse at all.
#[derive(PartialEq, Eq)]
struct Key {
    /// A hash of the whole source: any edit, same length or not, rebuilds the list.
    source: u64,
    query: String,
    opts: FindOptions,
}

/// One view's find state.
#[derive(Default)]
pub struct Find {
    pub open: bool,
    pub options: FindOptions,
    key: Option<Key>,
    matches: Vec<Match>,
    /// Index into [`Self::matches`]. Meaningless when the list is empty, and clamped into it
    /// whenever the list is rebuilt.
    current: usize,
    /// Set while nothing has been navigated to yet — a fresh query, a flipped option, a reopened
    /// bar. The first step then *reveals* the current match instead of stepping past it, which is
    /// what makes the first Enter land on the first hit rather than the second.
    fresh: bool,
    /// The bar's field, created on the first open: a `gpui_component` input needs a `Window`.
    input: Option<Entity<InputState>>,
    _watch: Option<Subscription>,
}

impl Find {
    /// Drop the list — the bar closed, or there is no document.
    pub fn forget(&mut self) {
        self.key = None;
        self.matches.clear();
        self.restart();
    }

    /// A fresh query or a flipped option: the next step lands on the first hit.
    pub fn restart(&mut self) {
        self.current = 0;
        self.fresh = true;
    }

    /// Bring the match list up to date with the document the frame is about to draw.
    ///
    /// Called on every frame, open or closed, because the closed case is the one that has to
    /// *drop* a stale list: rows would otherwise keep highlighting the hits of a query nobody can
    /// see.
    pub fn refresh(&mut self, doc: &Document, query: &str) {
        if !self.open {
            self.forget();
            return;
        }
        let key = Key {
            source: ubiq_md::content_hash(&doc.source),
            query: query.to_string(),
            opts: self.options,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.matches = search(doc, &key.query, key.opts);
        self.current = clamp(self.current, self.matches.len());
        self.key = Some(key);
    }

    /// Move to the next or previous match, wrapping at both ends. Answers the root block the
    /// current match is in — the `list()` row to reveal — or `None` with no matches.
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        let count = self.matches.len();
        if count == 0 {
            return None;
        }
        self.current = match (std::mem::take(&mut self.fresh), forward) {
            // Nothing navigated to yet: the first step lands on the first hit, and a backwards
            // first step on the last — the same reading of "wrap" from a standing start.
            (true, true) => 0,
            (true, false) => count - 1,
            (false, true) => next_match(self.current, count, true),
            (false, false) => prev_match(self.current, count, true),
        };
        Some(self.matches[self.current].root_block)
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// The hits inside root block `ix`, or `None` when the bar is shut or the row has none.
    pub fn hits(&self, ix: usize) -> Option<Hits> {
        if !self.open || self.matches.is_empty() {
            return None;
        }
        let mut ranges = Vec::new();
        let mut current = None;
        for (n, m) in self.matches.iter().enumerate() {
            if m.root_block != ix {
                continue;
            }
            if n == self.current {
                current = Some(ranges.len());
            }
            ranges.push(m.range.clone());
        }
        (!ranges.is_empty()).then_some(Hits { ranges, current })
    }

    /// "3 of 17", or what there is to say instead — what the find bar will show.
    pub fn position(&self) -> String {
        let empty = self
            .key
            .as_ref()
            .is_none_or(|key| key.query.trim().is_empty());
        match (empty, self.matches.len()) {
            (true, _) => String::new(),
            (false, 0) => "no matches".to_string(),
            (false, count) => format!("{} of {count}", self.current + 1),
        }
    }
}

/// The hits inside one row, in the terms the row draws in: source byte ranges, and which of them is
/// the current one.
pub struct Hits {
    pub ranges: Vec<Range<usize>>,
    /// Index into [`Self::ranges`], or `None` when the current match is in another row.
    pub current: Option<usize>,
}

thread_local! {
    /// The hits of the row being built, for as long as it is being built. See [`scope`].
    static ROW: RefCell<Option<Rc<Hits>>> = const { RefCell::new(None) };
}

// ── Navigation arithmetic ───────────────────────────────────────────

/// The next match after `current`, wrapping at the end when `wrap` is set and stopping at it
/// otherwise. `0` for an empty list, which is the only answer an empty list has.
pub fn next_match(current: usize, count: usize, wrap: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let current = current.min(count - 1);
    if current + 1 < count {
        current + 1
    } else if wrap {
        0
    } else {
        current
    }
}

/// The match before `current`, wrapping at the start when `wrap` is set.
pub fn prev_match(current: usize, count: usize, wrap: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let current = current.min(count - 1);
    match current.checked_sub(1) {
        Some(previous) => previous,
        None if wrap => count - 1,
        None => 0,
    }
}

/// The engine's scan. Here only so the query's emptiness is checked in one place.
fn search(doc: &Document, query: &str, opts: FindOptions) -> Vec<Match> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    doc.find(query, opts)
}

fn clamp(current: usize, count: usize) -> usize {
    match count {
        0 => 0,
        count => current.min(count - 1),
    }
}

// ── The bar ─────────────────────────────────────────────────────────

impl MdView {
    /// What is typed in the field, or nothing before the bar was ever opened.
    pub(super) fn find_query(&self, cx: &App) -> String {
        self.find
            .input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Open the bar (creating the field on first use) and put the caret in it.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = match self.find.input.clone() {
            Some(input) => input,
            None => {
                let input = cx
                    .new(|cx| InputState::new(window, cx).placeholder("Find in document\u{2026}"));
                let watch = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.find_changed(cx);
                    }
                });
                self.find.input = Some(input.clone());
                self.find._watch = Some(watch);
                input
            }
        };
        let was_open = self.find.open;
        self.find.open = true;
        input.update(cx, |state, cx| state.focus(window, cx));
        if was_open {
            // Already searching: refocus only, keep the current hit.
            cx.notify();
        } else {
            self.find_changed(cx);
        }
    }

    /// Close the bar, drop the highlights and hand the keyboard back to the page.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = false;
        self.find.forget();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A new query or a flipped facet: rescan, and land on the first hit.
    fn find_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query(cx);
        self.find.restart();
        self.find.refresh(&self.doc, &query);
        self.reveal_match(true, cx);
    }

    /// Step to the next or previous hit and scroll its row into view.
    pub fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if !self.find.open {
            return;
        }
        let query = self.find_query(cx);
        self.find.refresh(&self.doc, &query);
        self.reveal_match(forward, cx);
    }

    fn reveal_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(row) = self.find.step(forward) {
            self.list.scroll_to_reveal_item(row);
        }
        cx.notify();
    }

    /// The bar's actions, wired on the view's root.
    pub(super) fn find_actions(
        root: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        root.on_action(cx.listener(|this, _: &MdFindOpen, window, cx| this.open_find(window, cx)))
            .on_action(cx.listener(|this, _: &MdFindNext, _, cx| this.find_step(true, cx)))
            .on_action(cx.listener(|this, _: &MdFindPrev, _, cx| this.find_step(false, cx)))
            .on_action(cx.listener(|this, _: &MdFindClose, window, cx| this.close_find(window, cx)))
    }

    /// The floating bar, when open. `right` is how far in from the view's right edge it floats —
    /// past a minimap strip on that side.
    pub(super) fn find_bar(&self, right: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.find.input.clone().filter(|_| self.find.open)?;
        let position = self.find.position();
        let none = position == "no matches";
        let options = self.find.options;
        let size = theme::font(Family::Chrome, Role::Body);

        let step = |id: &'static str, icon: IconName, tip: &'static str, forward: bool| {
            crate::ui::kit::icon_button_tip(
                id,
                icon,
                tip,
                true,
                cx.listener(move |this, _, _, cx| this.find_step(forward, cx)),
            )
        };

        Some(
            div()
                .id("md-find")
                .key_context(KEY_CONTEXT)
                .absolute()
                .top(px(8.))
                .right(px(right))
                .h(px(30.))
                .flex()
                .items_center()
                .bg(theme::surface_raised())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent())
                .occlude()
                // A press inside the bar is the bar's: nothing under it hears it.
                .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                .text_size(size)
                .child(div().w(px(200.)).px_1().child(Input::new(&input).appearance(false)))
                .child(
                    div()
                        .min_w(px(64.))
                        .px_1()
                        .text_color(if none {
                            theme::danger()
                        } else {
                            theme::text_muted()
                        })
                        .child(position),
                )
                .child(facet(
                    "md-find-case",
                    "Aa",
                    "Match case",
                    options.case_sensitive,
                    cx.listener(|this, _, _, cx| {
                        this.find.options.case_sensitive = !this.find.options.case_sensitive;
                        this.find_changed(cx);
                    }),
                ))
                .child(facet(
                    "md-find-word",
                    "W",
                    "Whole word",
                    options.whole_word,
                    cx.listener(|this, _, _, cx| {
                        this.find.options.whole_word = !this.find.options.whole_word;
                        this.find_changed(cx);
                    }),
                ))
                .child(step("md-find-prev", IconName::ChevronUp, "Previous match", false))
                .child(step("md-find-next", IconName::ChevronDown, "Next match", true))
                .child(crate::ui::kit::icon_button_tip(
                    "md-find-close",
                    Icon::new(IconName::Close).with_size(Size::XSmall),
                    "Close",
                    true,
                    cx.listener(|this, _, window, cx| this.close_find(window, cx)),
                ))
                .into_any_element(),
        )
    }
}

/// One of the bar's facets: the bare glyph, lit when on and faint when off.
fn facet(
    id: &'static str,
    label: &'static str,
    tip: &'static str,
    on: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let tip: SharedString = tip.into();
    div()
        .id(id)
        .h_full()
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .cursor_pointer()
        .text_color(if on {
            theme::accent()
        } else {
            theme::text_faint()
        })
        .when(on, |this| this.bg(theme::accent_soft()))
        .hover(|this| this.bg(theme::hover()))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
        .on_click(on_click)
        .child(label)
}

// ── The row's highlight ─────────────────────────────────────────────

/// Publish the hits of root block `ix` for as long as the returned guard lives.
///
/// `blocks::row` holds one of these across the row it builds; `inline::runs` reads it through
/// [`row_hits`]. The guard is what keeps a row's hits from leaking into the next row, including on
/// the early-return paths a block kind can take. `find` is `None` for a view with no find state —
/// a read-only surface — which publishes nothing.
#[must_use = "the hits are published for as long as the guard lives"]
pub fn scope(find: Option<&Find>, ix: usize) -> RowScope {
    let hits = find.and_then(|find| find.hits(ix)).map(Rc::new);
    let published = hits.is_some();
    if published {
        ROW.with_borrow_mut(|row| *row = hits);
    }
    RowScope(published)
}

/// See [`scope`].
pub struct RowScope(bool);

impl Drop for RowScope {
    fn drop(&mut self) {
        if self.0 {
            ROW.with_borrow_mut(|row| *row = None);
        }
    }
}

/// The hits of the row currently being built, if it has any.
pub fn row_hits() -> Option<Rc<Hits>> {
    ROW.with_borrow(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forwards, wrapping: the last match steps to the first.
    #[test]
    fn next_wraps_at_the_end() {
        assert_eq!(next_match(0, 3, true), 1);
        assert_eq!(next_match(1, 3, true), 2);
        assert_eq!(next_match(2, 3, true), 0);
    }

    /// Backwards, wrapping: the first match steps to the last.
    #[test]
    fn prev_wraps_at_the_start() {
        assert_eq!(prev_match(2, 3, true), 1);
        assert_eq!(prev_match(1, 3, true), 0);
        assert_eq!(prev_match(0, 3, true), 2);
    }

    /// Without the wrap both ends hold, which is what a "stop at the end" mode would want.
    #[test]
    fn without_the_wrap_the_ends_hold() {
        assert_eq!(next_match(2, 3, false), 2);
        assert_eq!(prev_match(0, 3, false), 0);
    }

    /// One match: every step is a no-op, wrapping or not, and never an out-of-range index.
    #[test]
    fn a_single_match_steps_to_itself() {
        for wrap in [true, false] {
            assert_eq!(next_match(0, 1, wrap), 0);
            assert_eq!(prev_match(0, 1, wrap), 0);
        }
    }

    /// No matches: `0`, and no arithmetic that could underflow or index anything.
    #[test]
    fn no_matches_step_nowhere() {
        for wrap in [true, false] {
            assert_eq!(next_match(0, 0, wrap), 0);
            assert_eq!(prev_match(0, 0, wrap), 0);
            assert_eq!(next_match(7, 0, wrap), 0);
            assert_eq!(prev_match(7, 0, wrap), 0);
        }
    }

    /// A `current` left over from a longer list — which is what a reparse hands us — lands back
    /// inside the new one rather than panicking or answering out of range.
    #[test]
    fn a_stale_current_is_clamped() {
        assert_eq!(clamp(9, 3), 2);
        assert_eq!(clamp(9, 0), 0);
        assert_eq!(next_match(9, 3, true), 0);
        assert_eq!(prev_match(9, 3, true), 1);
    }

    /// The whole state path: refresh against a document, step through it, publish one row's hits
    /// through a scope and see them gone once the scope drops.
    #[test]
    fn a_query_steps_through_its_rows_and_scopes_one_row_at_a_time() {
        let doc = ubiq_md::parse("alpha beta\n\nbeta gamma\n\nnothing\n");
        let mut find = Find {
            open: true,
            ..Find::default()
        };
        find.refresh(&doc, "beta");
        find.restart();
        assert_eq!(find.count(), 2);
        assert_eq!(
            find.step(true),
            Some(0),
            "the first step lands on the first hit"
        );
        assert_eq!(find.position(), "1 of 2");
        assert_eq!(find.step(true), Some(1));
        assert_eq!(find.step(true), Some(0), "and wraps");

        {
            let _row = scope(Some(&find), 0);
            let hits = row_hits().expect("row 0 has a hit");
            assert_eq!(hits.ranges.len(), 1);
            assert_eq!(hits.current, Some(0));
        }
        assert!(row_hits().is_none(), "the scope took its hits with it");
        {
            let _row = scope(Some(&find), 2);
            assert!(row_hits().is_none(), "row 2 has no hit");
        }

        find.open = false;
        find.refresh(&doc, "beta");
        assert_eq!(find.count(), 0, "a shut bar drops its list");
        assert!(find.hits(0).is_none());
    }
}
