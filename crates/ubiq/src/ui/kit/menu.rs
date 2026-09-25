//! One dropdown mechanism, used by every menu in the window.
//!
//! The trigger renders in place and the list is painted through `deferred`, so it sits above the
//! rest of the shell and is dismissed by a click anywhere outside it. Which menu is open is the
//! caller's state, not the picker's — exactly one may be down at a time.

use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, RenderOnce, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{Action, IndexedAction, elided, field};

/// Where a dropdown is painted: above the shell, below a modal.
pub const MENU_LAYER: usize = 1;

/// Where a dropdown inside a modal is painted, over the modal's own overlay.
///
/// Public because a hand-built panel that hangs off a control inside a modal — the start form's
/// MCP checklist — has to be painted at the same rung as [`Picker::above_modal`] puts one, or it
/// opens underneath the panel holding it.
pub const MODAL_MENU_LAYER: usize = 3;

/// Anchor for a menu that must open upward, clear of the window's bottom edge.
pub const MENU_ANCHOR_UP: Anchor = Anchor::BottomLeft;

/// How the trigger is drawn. The titlebar's pickers read as plain text; the composer's read as
/// chips, because they sit on a busier surface.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerStyle {
    Plain,
    Chip,
    /// The shape [`crate::ui::kit::field`] gives a text input: a filled box on its own left edge,
    /// the value on the left and the chevron pinned to the right.
    ///
    /// For a picker in a form, where the plain trigger reads as a line of text rather than as
    /// something to click. It takes the width it is given, so a column of them lines up.
    Field,
}

#[derive(IntoElement)]
pub struct Picker {
    id: ElementId,
    icon: Option<IconName>,
    label: SharedString,
    items: Vec<SharedString>,
    /// Rows drawn but not pickable — a conversation already attached to another chat tab, say.
    /// Never a reason to drop a row from `items`: a row that vanishes reads as gone, not taken.
    disabled: Vec<usize>,
    /// Rows drawn as a hairline instead of text — a heading's own group line. Still occupies its
    /// index in `items`, the same rule `ContextItem::separator()` follows, so a caller building a
    /// list of decorations and real rows together need not renumber either.
    separators: Vec<usize>,
    /// Rows drawn muted, by index into `items` — a completed or abandoned mission on the board
    /// toolbar's mission filter, still pickable, unlike `disabled`.
    dim: Vec<usize>,
    /// A status colour per row, parallel to `items`. `None` at an index is a row with no dot of
    /// its own — [`MultiPicker::dots`]'s single-select counterpart, `Option`-valued because a
    /// single-choice list often mixes rows that carry one (a mission's phase) with rows that do
    /// not (an "All ..." row).
    dots: Vec<Option<Rgba>>,
    selected: Option<usize>,
    open: bool,
    anchor: Anchor,
    style: PickerStyle,
    on_toggle: Option<Action>,
    on_pick: Option<IndexedAction>,
    on_dismiss: Option<Action>,
    /// A filter field drawn at the top of the panel: the buffer, and whether it holds focus.
    /// `None` is every picker that has not opted in — see [`Self::search`].
    search: Option<(Entity<InputState>, bool)>,
    /// What the trigger says on hover. The one way a picker drawn with no label says what its
    /// list is about — see [`Self::tooltip`].
    tooltip: Option<SharedString>,
    layer: usize,
}

impl Picker {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            items: Vec::new(),
            disabled: Vec::new(),
            separators: Vec::new(),
            dim: Vec::new(),
            dots: Vec::new(),
            selected: None,
            open: false,
            anchor: Anchor::TopLeft,
            style: PickerStyle::Plain,
            on_toggle: None,
            on_pick: None,
            on_dismiss: None,
            search: None,
            tooltip: None,
            layer: MENU_LAYER,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn items<S: AsRef<str>>(mut self, items: impl IntoIterator<Item = S>) -> Self {
        self.items = items
            .into_iter()
            .map(|s| SharedString::from(s.as_ref().to_string()))
            .collect();
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// Mark rows drawn but not pickable, by index into `items`. The selected row stays pickable
    /// even if it is also passed here — a picker's own current value is never the one row it
    /// disables.
    pub fn disabled(mut self, indices: impl IntoIterator<Item = usize>) -> Self {
        self.disabled = indices.into_iter().collect();
        self
    }

    /// Mark rows drawn as a hairline instead of text, by index into `items` — a group heading's
    /// own separator, in the same list as the rows it groups. What `item` holds at that index is
    /// never read for it.
    pub fn separators(mut self, indices: impl IntoIterator<Item = usize>) -> Self {
        self.separators = indices.into_iter().collect();
        self
    }

    /// Mark rows drawn muted, by index into `items` — a mission past its work, still pickable,
    /// unlike [`Self::disabled`].
    pub fn dim(mut self, indices: impl IntoIterator<Item = usize>) -> Self {
        self.dim = indices.into_iter().collect();
        self
    }

    /// A status colour per row, in `items` order. `None` at an index draws that row with no dot.
    pub fn dots(mut self, dots: impl IntoIterator<Item = Option<Rgba>>) -> Self {
        self.dots = dots.into_iter().collect();
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Where the list hangs from the trigger. `BottomLeft` opens upward, which is what the
    /// composer needs.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn style(mut self, style: PickerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    pub fn on_pick(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(handler));
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Draw a filter field at the top of the panel. The caller has already filtered `items`;
    /// this only draws the field and reports the focus ring — the same split `project_menu`'s
    /// hand-rolled search uses.
    /// What the trigger says on hover.
    ///
    /// For the picker drawn as a bare chevron: with no label there is nothing on the control
    /// saying which question it asks, and the hover is where that goes rather than a word of text
    /// repeating what the row beside it already says.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn search(mut self, state: &Entity<InputState>, focused: bool) -> Self {
        self.search = Some((state.clone(), focused));
        self
    }

    /// Raise the list over a modal.
    ///
    /// A dropdown is painted at [`MENU_LAYER`], which is above the shell and **below** the layer
    /// [`crate::ui::kit::modal`] paints at — so a picker inside a modal would open underneath the
    /// panel holding it, which reads as a control that does nothing. A picker in a modal says so.
    pub fn above_modal(mut self) -> Self {
        self.layer = MODAL_MENU_LAYER;
        self
    }
}

impl RenderOnce for Picker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Picker {
            id,
            icon,
            label,
            items,
            disabled,
            separators,
            dim,
            dots,
            selected,
            open,
            anchor,
            style,
            on_toggle,
            on_pick,
            on_dismiss,
            search,
            tooltip,
            layer,
        } = self;

        let panel_id = ElementId::Name(format!("{id:?}-menu").into());

        // The value gives way rather than pushing the chevron off the end: a field-shaped trigger
        // is one line high, and a long model id is what would otherwise widen the whole column.
        let value = match style {
            PickerStyle::Field => div().flex_1().min_w(px(0.)).truncate().child(label),
            _ => div().child(label),
        };
        let mut trigger = trigger_shell(id, style, icon, on_toggle)
            .child(value)
            .child(chevron());

        if let Some(text) = tooltip {
            trigger = trigger.tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
            });
        }

        if open {
            let rows = items
                .into_iter()
                .enumerate()
                .map(|(ix, label)| PanelRow {
                    index: ix,
                    label,
                    // A picker's own current value is never the row it disables, whatever the
                    // caller passed — the one row a picker cannot let you leave unpicked is the
                    // one already picked.
                    selected: selected == Some(ix),
                    disabled: selected != Some(ix) && disabled.contains(&ix),
                    separator: separators.contains(&ix),
                    dim: dim.contains(&ix),
                    dot: dots.get(ix).copied().flatten(),
                })
                .collect();
            trigger = trigger.child(menu_panel(
                panel_id, anchor, rows, on_pick, None, on_dismiss, search, layer,
            ));
        }

        trigger
    }
}

/// The trigger every dropdown in the window is drawn on: the box, its hover, its style and the
/// click that opens the list. What goes *inside* it — a value, a comma-separated summary — is the
/// caller's, appended after this returns.
fn trigger_shell(
    id: ElementId,
    style: PickerStyle,
    icon: Option<IconName>,
    on_toggle: Option<Action>,
) -> Stateful<Div> {
    let mut trigger = div()
        .id(id)
        .relative()
        .h(px(26.))
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .text_size(theme::font(Family::Chrome, Role::Body))
        .text_color(theme::text())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()));

    match style {
        PickerStyle::Chip => {
            trigger = trigger
                .px_2()
                .bg(theme::surface())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::border());
        }
        PickerStyle::Field => {
            trigger = trigger
                .h(px(28.))
                .w_full()
                .px_2()
                .justify_between()
                .bg(theme::surface())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::border());
        }
        PickerStyle::Plain => trigger = trigger.px_2(),
    }

    if let Some(icon) = icon {
        trigger = trigger.child(
            Icon::new(icon)
                .with_size(Size::XSmall)
                .text_color(theme::text_muted()),
        );
    }

    if let Some(toggle) = on_toggle {
        // Opening rather than toggling: the panel's own outside-click dismissal would
        // otherwise race this click into reopening a menu the user meant to close.
        trigger = trigger.on_click(move |_, window, cx| toggle(window, cx));
    }

    trigger
}

fn chevron() -> Icon {
    Icon::new(IconName::ChevronDown)
        .with_size(Size::XSmall)
        .text_color(theme::text_faint())
}

/// One row a dropdown draws, already resolved: the index the caller knows it by, what it says,
/// and how it is dressed.
///
/// **The index is the row's place in the caller's `items`, not its place on screen.** A
/// [`MultiPicker`] draws its selected rows first, and this is what keeps `on_pick` answering in
/// the numbering the caller handed in.
struct PanelRow {
    index: usize,
    label: SharedString,
    selected: bool,
    disabled: bool,
    separator: bool,
    /// Drawn muted, but still clickable — unlike `disabled`, which also refuses the click. A
    /// finished mission on the board toolbar's mission filter.
    dim: bool,
    /// A filled dot before the label, for a list whose values carry a status colour of their own
    /// — the states filter, where the colour is half of what the row says. The same 7px dot
    /// [`crate::ui::kit::toggle_pill`] wears, so a filter moved off a pill row into a menu reads
    /// the same way.
    dot: Option<Rgba>,
}

fn menu_panel(
    id: ElementId,
    anchor: Anchor,
    rows: Vec<PanelRow>,
    on_pick: Option<IndexedAction>,
    on_select: Option<(SharedString, IndexedAction)>,
    on_dismiss: Option<Action>,
    search: Option<(Entity<InputState>, bool)>,
    layer: usize,
) -> impl IntoElement {
    // The caller has already filtered its items — an empty result is said, once, rather than left
    // as a panel with nothing in it.
    let rows: Vec<AnyElement> = if rows.is_empty() {
        vec![
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_faint())
                .child("No matches")
                .into_any_element(),
        ]
    } else {
        rows.into_iter()
            .map(|item| {
                let ix = item.index;
                if item.separator {
                    return div()
                        .id(("menu-separator", ix))
                        .my_1()
                        .h(px(1.))
                        .flex_none()
                        .bg(theme::border())
                        .into_any_element();
                }
                let is_selected = item.selected;
                let is_disabled = item.disabled;
                let is_dim = item.dim;
                let pick = on_pick.clone();
                let mut row = div()
                    .id(("menu-row", ix))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(if is_disabled || (is_dim && !is_selected) {
                        theme::text_faint()
                    } else if is_selected {
                        theme::text()
                    } else {
                        theme::text_muted()
                    })
                    .child(div().w(px(12.)).flex().flex_none().justify_center().child(
                        if is_selected {
                            Icon::new(IconName::Check)
                                .with_size(Size::XSmall)
                                .text_color(theme::accent())
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    ))
                    .children(
                        item.dot
                            .map(|colour| div().size(px(7.)).flex_none().rounded_full().bg(colour)),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(item.label));
                if !is_disabled {
                    row = row
                        .cursor_pointer()
                        .hover(|this| this.bg(theme::hover()).text_color(theme::text()))
                        .on_click(move |_, window, cx| {
                            if let Some(pick) = pick.clone() {
                                pick(ix, window, cx);
                            }
                        });
                    // A second target, beside the toggle: points the screen at this one row
                    // rather than narrowing the set. `stop_propagation` keeps the click off the
                    // row's own `on_pick`, which would otherwise also fire underneath it.
                    if let Some((tooltip, select)) = on_select.clone() {
                        row = row.child(
                            div()
                                .id(("menu-row-select", ix))
                                .size(px(20.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|this| this.bg(theme::hover()))
                                .child(
                                    Icon::new(IconName::ArrowRight)
                                        .with_size(Size::XSmall)
                                        .text_color(theme::text_faint()),
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    select(ix, window, cx);
                                })
                                .tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(tooltip.clone())
                                        .build(window, cx)
                                }),
                        );
                    }
                }
                row.into_any_element()
            })
            .collect()
    };

    let search_field = search.map(|(state, focused)| {
        field(theme::accent(), focused)
            .h(px(28.))
            .px_2()
            .flex_none()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
            .child(
                Icon::new(IconName::Search)
                    .with_size(Size::XSmall)
                    .text_color(theme::text_faint()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .child(Input::new(&state).appearance(false)),
            )
    });

    deferred(
        anchored()
            .anchor(anchor)
            .snap_to_window_with_margin(px(8.))
            .child(
                div()
                    .id(id)
                    .min_w(px(180.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .bg(theme::surface_raised())
                    .border_l(px(theme::accent_edge()))
                    .border_color(theme::accent())
                    .shadow_lg()
                    .font_weight(FontWeight::NORMAL)
                    .children(search_field)
                    .children(rows)
                    // The list is painted above whatever it hangs from — a modal's own body
                    // included, per `ui/kit/overlay.rs` — so a click on a row sits at the same
                    // screen point as a control underneath. Stopping it here is what keeps it
                    // from also landing on that control, the way a dock resize strip already
                    // consumes its own mouse-down.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down_out(move |_, window, cx| {
                        if let Some(dismiss) = on_dismiss.clone() {
                            dismiss(window, cx);
                        }
                    }),
            ),
    )
    .priority(layer)
}

/// How wide a closed [`MultiPicker`] lets its summary grow before eliding it, at `ui_scale = 1.0`.
///
/// A chip on a toolbar has no width of its own — it is as wide as what it says — so a control
/// whose label is "every value you ticked" needs a ceiling, or four choices push the rest of the
/// row off the screen. `PickerStyle::Field` ignores it and fills its column instead.
pub const MULTI_WIDTH: f32 = 170.0;

/// The [`Picker`]'s multi-select sibling: the same trigger, the same list, the same `MenuId`
/// discipline — but a row **toggles** instead of answering, and the menu stays down until it is
/// dismissed.
///
/// Closed, it says what is ticked: the values comma-separated, elided to [`MULTI_WIDTH`] with the
/// whole list on the hover (`kit::elided`), or the placeholder when nothing is. The selection goes
/// **in** as well as out — `selected` is a set of indices into `items`, so a filter hands it what
/// is currently narrowing the screen and a form hands it what the record already holds, and the
/// control is the same either way.
///
/// It owns no state. `on_pick(index)` is called with the caller's own index and the caller decides
/// what ticking means; nothing here closes the menu.
#[derive(IntoElement)]
pub struct MultiPicker {
    id: ElementId,
    icon: Option<IconName>,
    /// What a closed trigger says with nothing ticked. For a filter that is what an empty
    /// selection *shows* — "all states" — rather than the word "none".
    placeholder: SharedString,
    items: Vec<SharedString>,
    /// Indices into `items`, in any order: what is already ticked.
    selected: Vec<usize>,
    /// A status colour per row, parallel to `items`. Empty for a list whose values have none.
    dots: Vec<Rgba>,
    open: bool,
    anchor: Anchor,
    style: PickerStyle,
    on_toggle: Option<Action>,
    on_pick: Option<IndexedAction>,
    /// A second click target per row, beside the one that toggles it — for a set-valued list whose
    /// rows are also individually worth landing on, where toggling a filter and pointing the
    /// screen at one row are two different questions. `None` is the ordinary multi-select, every
    /// row answering only `on_pick`.
    on_select: Option<(SharedString, IndexedAction)>,
    on_dismiss: Option<Action>,
    search: Option<(Entity<InputState>, bool)>,
    layer: usize,
}

impl MultiPicker {
    pub fn new(id: impl Into<ElementId>, placeholder: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            placeholder: placeholder.into(),
            items: Vec::new(),
            selected: Vec::new(),
            dots: Vec::new(),
            open: false,
            anchor: Anchor::TopLeft,
            style: PickerStyle::Chip,
            on_toggle: None,
            on_pick: None,
            on_select: None,
            on_dismiss: None,
            search: None,
            layer: MENU_LAYER,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn items<S: AsRef<str>>(mut self, items: impl IntoIterator<Item = S>) -> Self {
        self.items = items
            .into_iter()
            .map(|s| SharedString::from(s.as_ref().to_string()))
            .collect();
        self
    }

    /// What is already ticked, by index into `items`. The preselection *is* the value: nothing
    /// here remembers a pick, so a caller that does not pass its current set back on the next
    /// frame has an empty control.
    pub fn selected(mut self, indices: impl IntoIterator<Item = usize>) -> Self {
        self.selected = indices.into_iter().collect();
        self
    }

    /// A status colour per row, in `items` order.
    pub fn dots(mut self, dots: impl IntoIterator<Item = Rgba>) -> Self {
        self.dots = dots.into_iter().collect();
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn style(mut self, style: PickerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    /// Called with the index of the row that was clicked, in the caller's `items` numbering —
    /// never the row's position on screen, which the selected-first order moves.
    pub fn on_pick(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(handler));
        self
    }

    /// A trailing icon on each row, beside the toggle, that points the screen at that one row
    /// instead of narrowing the set — `tooltip` names what it does, since the icon carries no
    /// label of its own. Called with the index of the row picked, in the caller's `items`
    /// numbering, exactly as [`Self::on_pick`] is.
    pub fn on_select(
        mut self,
        tooltip: impl Into<SharedString>,
        handler: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some((tooltip.into(), Rc::new(handler)));
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Draw a filter field at the top of the panel. As with [`Picker::search`] the caller has
    /// already narrowed `items`; the field is drawn here and the query is read back off the
    /// buffer to decide whether the ticked rows are pinned to the top.
    pub fn search(mut self, state: &Entity<InputState>, focused: bool) -> Self {
        self.search = Some((state.clone(), focused));
        self
    }

    /// Raise the list over a modal — [`Picker::above_modal`]'s reason, unchanged.
    pub fn above_modal(mut self) -> Self {
        self.layer = MODAL_MENU_LAYER;
        self
    }
}

/// What a closed multi-select says: the ticked values in `items` order, comma-separated, or the
/// placeholder when nothing is ticked.
///
/// `items` order rather than the order they were ticked in — a label that reshuffles as you tick
/// is a label nobody can read twice. Indices that name no row are dropped: the caller's selection
/// and its list are two values and a frame can arrive holding an old one.
pub fn multi_label(
    items: &[SharedString],
    selected: &[usize],
    placeholder: &SharedString,
) -> SharedString {
    let picked: Vec<&str> = items
        .iter()
        .enumerate()
        .filter(|(ix, _)| selected.contains(ix))
        .map(|(_, label)| label.as_ref())
        .collect();
    if picked.is_empty() {
        return placeholder.clone();
    }
    SharedString::from(picked.join(", "))
}

/// The order a multi-select draws its rows in: indices into the caller's `items`.
///
/// **Unfiltered, the ticked rows come first** — in `items` order, then everything left in `items`
/// order — because a list whose selection is scattered down it takes reading to answer "what did I
/// choose".
///
/// **Under a query, nothing is pinned.** The list is the search result in the search's own order:
/// a ticked row lifted above better matches reads as a match that it is not, and the user typed to
/// find something. `None` is a list with no search field at all, which has no query to be empty
/// and keeps the order the caller chose — a short fixed list whose rows jump as they are ticked is
/// harder to use, not easier.
pub fn multi_order(len: usize, selected: &[usize], query: Option<&str>) -> Vec<usize> {
    if query != Some("") {
        return (0..len).collect();
    }
    let (ticked, rest): (Vec<usize>, Vec<usize>) = (0..len).partition(|ix| selected.contains(ix));
    ticked.into_iter().chain(rest).collect()
}

impl RenderOnce for MultiPicker {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let MultiPicker {
            id,
            icon,
            placeholder,
            items,
            selected,
            dots,
            open,
            anchor,
            style,
            on_toggle,
            on_pick,
            on_select,
            on_dismiss,
            search,
            layer,
        } = self;

        let panel_id = ElementId::Name(format!("{id:?}-menu").into());
        let label_id = ElementId::Name(format!("{id:?}-label").into());
        let label = multi_label(&items, &selected, &placeholder);

        // The summary elides rather than wraps, and carries itself on the hover — which is the
        // whole list of what is ticked, said in full, exactly where a truncated one stops saying
        // it.
        let mut value = elided(
            label_id,
            label,
            theme::text(),
            theme::font(Family::Chrome, Role::Body),
        );
        if style != PickerStyle::Field {
            value = value.max_w(px(theme::scaled(MULTI_WIDTH)));
        }

        let mut trigger = trigger_shell(id, style, icon, on_toggle)
            .child(value)
            .child(chevron());

        if open {
            // The query is read off the field rather than passed in: the caller has already
            // filtered `items` with it, so asking for it twice is one more thing to get wrong.
            let query = search
                .as_ref()
                .map(|(state, _)| state.read(cx).value().to_string());
            let rows = multi_order(items.len(), &selected, query.as_deref())
                .into_iter()
                .map(|ix| PanelRow {
                    index: ix,
                    label: items[ix].clone(),
                    selected: selected.contains(&ix),
                    disabled: false,
                    separator: false,
                    dim: false,
                    dot: dots.get(ix).copied(),
                })
                .collect();
            trigger = trigger.child(menu_panel(
                panel_id, anchor, rows, on_pick, on_select, on_dismiss, search, layer,
            ));
        }

        trigger
    }
}

/// One row in a context menu. `enabled` is what a prepared action that has nothing behind it yet
/// looks like: the wording is there, the click is not.
#[derive(Clone)]
pub struct ContextItem {
    pub label: SharedString,
    pub enabled: bool,
    /// A hairline instead of a row. It still occupies an index, because a menu's rows and the
    /// actions behind them are matched by position.
    pub separator: bool,
    /// A glyph drawn before the row's label, from either icon set. The label is drawn either
    /// way: the icon anchors a row, it never stands in for its name.
    pub icon: Option<Icon>,
    /// What the row cannot say in its label. A menu row is one short verb wide — a file path, a
    /// full account name, the reason a row is dead — is longer than the panel, so it hovers
    /// instead of widening every row to fit the one. Not a second label: a row whose *name* is in
    /// its tooltip is a row nobody reads.
    pub tooltip: Option<SharedString>,
}

impl ContextItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            separator: false,
            icon: None,
            tooltip: None,
        }
    }

    /// Drawn, and does nothing: the predisposition for an action the host does not answer yet.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Give this row a glyph before its name, for a menu whose commands are easier to pick out by
    /// picture than by reading down the column.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Hang on this row the fact its label has no room for — the path a dump is being written to,
    /// where the row can only afford to say `Stop dumping`. Shown on hover whether or not the row
    /// is enabled: a dead row is often the one a reader most wants explained.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// The line between two groups of rows — what tells "start this" from "show me that".
    pub fn separator() -> Self {
        Self {
            label: SharedString::default(),
            enabled: false,
            separator: true,
            icon: None,
            tooltip: None,
        }
    }
}

/// A menu that opens where the pointer went down, not from a trigger.
///
/// Painted through `deferred` and `anchored` so a right-click inside a dock panel covers the window
/// rather than being clipped to the panel. Dismissal is an outside click, the same as the
/// dropdown: the two behave the same way and neither uses a scrim.
pub fn context_menu(
    id: impl Into<ElementId>,
    at: Point<Pixels>,
    items: Vec<ContextItem>,
    on_pick: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    deferred(
        anchored()
            .position(at)
            .snap_to_window_with_margin(px(8.))
            .child(context_panel(
                id,
                items,
                Some(Rc::new(on_pick)),
                Some(Rc::new(on_dismiss)),
            )),
    )
    .priority(1)
}

/// The panel a context menu is. Exported so the style reference can draw one in place, without
/// the deferred layer a real right-click needs.
pub fn context_panel(
    id: impl Into<ElementId>,
    items: Vec<ContextItem>,
    on_pick: Option<IndexedAction>,
    on_dismiss: Option<Action>,
) -> impl IntoElement {
    let rows: Vec<_> = items
        .into_iter()
        .enumerate()
        .map(|(ix, item)| {
            if item.separator {
                return div()
                    .id(("menu-separator", ix))
                    .my_1()
                    .h(px(1.))
                    .flex_none()
                    .bg(theme::border());
            }
            let pick = on_pick.clone();
            let enabled = item.enabled;
            let mut row = div()
                .id(("menu-row", ix))
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(if enabled {
                    theme::text()
                } else {
                    theme::text_faint()
                });
            // The glyph sits *before* the name rather than instead of it: these rows were moved
            // off an icon strip precisely so they would read as words, and a column of unlabelled
            // glyphs is that strip again with the names hidden behind a hover.
            if let Some(icon) = item.icon {
                row = row.gap_2().child(
                    icon.with_size(Size::Size(theme::icon_md()))
                        .text_color(theme::text_muted()),
                );
            }
            row = row.child(item.label);

            // Hung on the row itself rather than on the label, so hovering anywhere across the
            // full width — including the dead part left of a short verb — answers it. The row
            // already carries an `.id`, which is what a tooltip needs.
            if let Some(tooltip) = item.tooltip {
                row = row.tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                });
            }

            if enabled {
                row = row
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()).text_color(theme::text()));
                if let Some(pick) = pick {
                    row = row.on_click(move |_, window, cx| pick(ix, window, cx));
                }
            }

            row
        })
        .collect();

    div()
        .id(id)
        .min_w(px(180.))
        .p_1()
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        .font_weight(FontWeight::NORMAL)
        .children(rows)
        // Same reason as the dropdown list in `menu_panel`: painted above whatever raised it, so
        // a click here must not also land on a control underneath.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(move |_, window, cx| {
            if let Some(dismiss) = on_dismiss.clone() {
                dismiss(window, cx);
            }
        })
}
