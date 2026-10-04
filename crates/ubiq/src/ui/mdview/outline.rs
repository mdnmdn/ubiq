//! The navigator: draws any [`Structure`] as a list of rows, and as the searchable jump popover.
//!
//! Both presentations share [`rows`]. The caller owns the state — a [`Jump`] field on whatever
//! entity hosts the popover — and wires it through [`JumpHooks`], three fn pointers saying how to
//! reach the `Jump` from the host, what activating a [`Target`] does, and what to do once the
//! popover has closed (hand focus back). Nothing here is thread-local and nothing names `AppState`.
//!
//! **The popover owns the mouse while it is up.** It is a `deferred(anchored(..))` layer the size
//! of the window: a transparent backdrop that `occlude`s — so nothing beneath is hovered, clicked
//! or scrolled — closes on any mouse down and stops scroll-wheel propagation; the panel on it
//! occludes too and stops its own mouse downs so they do not reach the backdrop.
//!
//! Keys: `Escape`, `up`, `down` and `enter` are bound for `MdJump > Input` by [`key_bindings`],
//! installed after the component library's own `Input` bindings (`app::install_key_bindings`) so
//! inside the filter field ours win.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement as _, KeyBinding, MouseButton, ParentElement as _, ScrollHandle, ScrollWheelEvent,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, actions,
    anchored, deferred, div, point, px,
};
use gpui_component::input::{Input, InputEvent, InputState};

use super::structure::{self, Structure, Target};
use crate::theme::{self, Family, Role};

actions!(md_jump, [MdJumpClose, MdJumpUp, MdJumpDown, MdJumpConfirm]);

/// The panel's key context, and the predicate the list keys are bound under.
const KEY_CONTEXT: &str = "MdJump";
const KEY_PREDICATE: &str = "MdJump > Input";

const WIDTH: f32 = 440.0;
const MAX_HEIGHT: f32 = 420.0;

/// The popover's keys, for `app::install_key_bindings` to register after the library's own.
pub fn key_bindings() -> [KeyBinding; 4] {
    [
        KeyBinding::new("escape", MdJumpClose, Some(KEY_PREDICATE)),
        KeyBinding::new("up", MdJumpUp, Some(KEY_PREDICATE)),
        KeyBinding::new("down", MdJumpDown, Some(KEY_PREDICATE)),
        KeyBinding::new("enter", MdJumpConfirm, Some(KEY_PREDICATE)),
    ]
}

// ── The rows ────────────────────────────────────────────────────────

/// The rows for `visible` (indices into the structure's items), shared by the dock-style list and
/// the popover.
///
/// `selected` is a position in `visible` — the popover's keyboard cursor; a plain list has none.
/// `on_pick` is what a click does, handed the item's [`Target`].
pub fn rows(
    structure: &dyn Structure,
    visible: &[usize],
    selected: Option<usize>,
    id: &'static str,
    on_pick: impl Fn(Target, &mut Window, &mut App) + Clone + 'static,
) -> Vec<AnyElement> {
    let items = structure.items();
    let current = structure.current();
    visible
        .iter()
        .enumerate()
        .map(|(pos, &n)| {
            let item = &items[n];
            let target = item.target;
            let on_pick = on_pick.clone();
            row(
                (id, n),
                item.depth,
                &item.label,
                Some(n) == current,
                Some(pos) == selected,
            )
            .on_click(move |_: &ClickEvent, window, cx| on_pick(target, window, cx))
            .into_any_element()
        })
        .collect()
}

/// A scrollable list of every item in `structure`, or its empty state.
pub fn list(
    structure: &dyn Structure,
    id: &'static str,
    on_pick: impl Fn(Target, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    if structure.items().is_empty() {
        return note(&format!("No {}", structure.title()));
    }
    let all: Vec<usize> = (0..structure.items().len()).collect();
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .py_1()
        .children(rows(structure, &all, None, id, on_pick))
        .into_any_element()
}

/// One item: indented by depth, the two top levels in the heading colour so the shape of the
/// structure is legible before any of the words are read.
fn row(
    id: (&'static str, usize),
    depth: u8,
    label: &str,
    current: bool,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    let indent = px(6.0 + f32::from(depth) * 10.0);

    div()
        .id(id)
        .flex()
        .items_center()
        .pl(indent)
        .pr_2()
        .py_0p5()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .when(depth <= 1, |this| this.text_color(theme::text_strong()))
        .when(depth > 1, |this| this.text_color(theme::text_muted()))
        // The section the preview is inside: a band and an accent edge, the same pair the tab
        // strip marks its active tab with.
        .when(current, |this| {
            this.bg(theme::accent_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent())
                .pl(px(f32::from(indent) - theme::ACCENT_EDGE))
                .text_color(theme::link_underline())
        })
        // The popover's keyboard cursor, drawn over the current band when they coincide.
        .when(selected, |this| {
            this.bg(theme::selected()).text_color(theme::text())
        })
        .hover(|style| style.bg(theme::hover()))
        .cursor_pointer()
        .child(SharedString::from(label.to_string()))
}

fn note(text: &str) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(theme::text_faint())
        .child(text.to_string())
        .into_any_element()
}

// ── The jump popover ────────────────────────────────────────────────

/// The popover's state, owned by the host entity `T` as a field.
#[derive(Default)]
pub struct Jump {
    open: bool,
    /// Created on the first open: a `gpui_component` input needs a `Window`.
    input: Option<Entity<InputState>>,
    /// Turns a keystroke in the field into a notify, so the list follows the query.
    _watch: Option<Subscription>,
    /// The keyboard cursor: a position in the *filtered* list.
    selected: usize,
    scroll: ScrollHandle,
    /// What the last overlay drew: how many rows match, and the target under the cursor — the
    /// key actions run between renders and answer from these.
    drawn: usize,
    picked: Option<Target>,
}

impl Jump {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The filtered list for the current query, as indices into the structure's items.
    fn visible(&self, structure: &dyn Structure, cx: &App) -> Vec<usize> {
        let query = self.input.as_ref().map(|input| input.read(cx).value());
        structure::filter(structure, query.as_deref().unwrap_or(""))
    }

    /// Move the cursor one row, wrapping.
    fn step(&mut self, forward: bool, count: usize) {
        if count == 0 {
            return;
        }
        let at = self.selected.min(count - 1);
        self.selected = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        self.scroll.scroll_to_item(self.selected);
    }
}

/// How the popover reaches its host `T`. Plain fn pointers, so the struct is `Copy` and every
/// listener closure takes it by value.
pub struct JumpHooks<T: 'static> {
    /// The host's `Jump` field.
    pub jump: fn(&mut T) -> &mut Jump,
    /// What activating a target does (`MdView::reveal_block` for a [`Target::Block`]). The popover
    /// is already closed when this runs.
    pub pick: fn(&mut T, Target, &mut Window, &mut Context<T>),
    /// After the popover closes by any route: hand focus back to the window so its keys work.
    pub closed: fn(&mut T, &mut Window, &mut Context<T>),
}

impl<T: 'static> Clone for JumpHooks<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for JumpHooks<T> {}

impl<T: 'static> JumpHooks<T> {
    /// Open the popover on a fresh query, creating the field on first use.
    pub fn open(self, this: &mut T, window: &mut Window, cx: &mut Context<T>) {
        let existing = (self.jump)(this).input.clone();
        let input = match existing {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Go to…"));
                let hooks = self;
                let watch = cx.subscribe(&input, move |this: &mut T, _, event: &InputEvent, cx| {
                    if let InputEvent::Change = event {
                        (hooks.jump)(this).selected = 0;
                        cx.notify();
                    }
                });
                let jump = (self.jump)(this);
                jump.input = Some(input.clone());
                jump._watch = Some(watch);
                input
            }
        };
        let jump = (self.jump)(this);
        jump.open = true;
        jump.selected = 0;
        // A fresh query each time: the popover is ephemeral, and a stale filter hides the list.
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Open when closed, close when open.
    pub fn toggle(self, this: &mut T, window: &mut Window, cx: &mut Context<T>) {
        if (self.jump)(this).open {
            self.close(this, window, cx);
        } else {
            self.open(this, window, cx);
        }
    }

    pub fn close(self, this: &mut T, window: &mut Window, cx: &mut Context<T>) {
        (self.jump)(this).close();
        (self.closed)(this, window, cx);
        cx.notify();
    }

    /// Go there and get out of the way.
    fn choose(self, this: &mut T, target: Target, window: &mut Window, cx: &mut Context<T>) {
        self.close(this, window, cx);
        (self.pick)(this, target, window, cx);
    }

    /// The overlay, when open: the occluding backdrop with the panel on it. `structure` is built
    /// by the caller from its document each frame (`Outline::new`); `None` shows an empty note.
    pub fn overlay(
        self,
        this: &mut T,
        structure: Option<&dyn Structure>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Option<AnyElement> {
        let jump = (self.jump)(this);
        let input = jump.input.clone().filter(|_| jump.open)?;
        let selected = jump.selected;
        let scroll = jump.scroll.clone();

        let mut drawn = 0;
        let mut picked = None;
        let body: Vec<AnyElement> = match structure {
            None => vec![note("No document open")],
            Some(structure) => {
                let visible = (self.jump)(this).visible(structure, cx);
                if visible.is_empty() {
                    vec![note(&format!("No matching {}", structure.title()))]
                } else {
                    let selected = selected.min(visible.len() - 1);
                    drawn = visible.len();
                    picked = Some(structure.items()[visible[selected]].target);
                    let hooks = self;
                    let entity = cx.entity();
                    rows(
                        structure,
                        &visible,
                        Some(selected),
                        "md-jump-row",
                        move |target, window, cx| {
                            entity.update(cx, |this, cx| hooks.choose(this, target, window, cx));
                        },
                    )
                }
            }
        };

        let jump = (self.jump)(this);
        jump.drawn = drawn;
        jump.picked = picked;

        let hooks = self;
        let panel = div()
            .id("md-jump-panel")
            .key_context(KEY_CONTEXT)
            .on_action(
                cx.listener(move |this, _: &MdJumpClose, window, cx| hooks.close(this, window, cx)),
            )
            .on_action(cx.listener(move |this, _: &MdJumpUp, _, cx| {
                let count = (hooks.jump)(this).drawn;
                (hooks.jump)(this).step(false, count);
                cx.notify();
            }))
            .on_action(cx.listener(move |this, _: &MdJumpDown, _, cx| {
                let count = (hooks.jump)(this).drawn;
                (hooks.jump)(this).step(true, count);
                cx.notify();
            }))
            .on_action(cx.listener(move |this, _: &MdJumpConfirm, window, cx| {
                match (hooks.jump)(this).picked {
                    Some(target) => hooks.choose(this, target, window, cx),
                    None => hooks.close(this, window, cx),
                }
            }))
            .occlude()
            // A press inside the panel is the panel's; without this the backdrop would close it.
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col()
            .w(px(WIDTH))
            .max_h(px(MAX_HEIGHT))
            .bg(theme::surface_raised())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::accent())
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .p_2()
                    .border_b(px(theme::hairline()))
                    .border_color(theme::border())
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .child(Input::new(&input)),
            )
            .child(
                div()
                    .id("md-jump-rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .py_1()
                    .children(body),
            );

        let viewport = window.viewport_size();
        let backdrop = div()
            .id("md-jump-backdrop")
            .w(viewport.width)
            .h(viewport.height)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(theme::TITLEBAR_HEIGHT + 8.0))
            // The capture: nothing beneath is hovered or hit while this is up, a press anywhere off
            // the panel dismisses, and the wheel stops here rather than scrolling the page under it.
            .occlude()
            .on_any_mouse_down(cx.listener(move |this, _, window, cx| {
                hooks.close(this, window, cx);
                cx.stop_propagation();
            }))
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_: &ScrollWheelEvent, _, cx| cx.stop_propagation())
            .child(panel);

        Some(
            deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
                // Above the component library's own dropdowns, which sit at 1.
                .priority(2)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::mdview::structure::Outline;

    #[test]
    fn the_cursor_wraps_both_ways() {
        let mut jump = Jump::default();
        jump.step(true, 3);
        assert_eq!(jump.selected, 1);
        jump.step(true, 3);
        jump.step(true, 3);
        assert_eq!(
            jump.selected, 0,
            "forward from the last row wraps to the first"
        );
        jump.step(false, 3);
        assert_eq!(
            jump.selected, 2,
            "back from the first row wraps to the last"
        );
        // No rows: nothing moves, nothing divides by zero.
        jump.step(true, 0);
        assert_eq!(jump.selected, 2);
    }

    #[test]
    fn a_stale_cursor_is_clamped_before_it_moves() {
        let mut jump = Jump {
            selected: 9,
            ..Default::default()
        };
        jump.step(true, 3);
        assert_eq!(
            jump.selected, 0,
            "a cursor past the shortened list steps from its end"
        );
    }

    #[test]
    fn the_popover_binds_its_four_keys_under_the_input_predicate() {
        assert_eq!(key_bindings().len(), 4);
        let outline = Outline::new(&ubiq_md::parse("# A\n\n## B\n"), None);
        assert_eq!(structure::filter(&outline, "b"), [1]);
    }
}
