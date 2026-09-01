//! Ubiq's appearance for the dock.
//!
//! The library owns the tree, the drag, the drop geometry and the persistence, and draws none of
//! it: every pixel comes through three renderer traits. So the dock is adopted without taking a
//! second house style with it — `D18` holds inside a group exactly as it does outside one. Square
//! surfaces, one coloured edge, tokens only, and the tab strip the editor and the old dock already
//! agreed on.
//!
//! Nothing here names `AppState`. It draws a tab from what the panel answers, which is what keeps
//! the skin a skin.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Div, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::dock::{
    BasePanelView, DockAreaRenderer, DockContext, DockPlacement, DropIndicator, TabGroupContext,
    TabGroupRenderer, TileContext, TilesRenderer,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::ui::dock::WorkbenchPanel;

/// The tab strip's height, shared with `ui::kit::tab_strip` so a dock tab and an editor tab sit on
/// the same line.
const TAB_BAR: f32 = 38.0;

/// The hit area of the strip that resizes a region, and the hairline drawn inside it. Wide enough
/// to grab, drawn as an edge rather than a bar.
const RESIZE_STRIP: f32 = 4.0;

/// What a panel's tab says. Recovered across the renderer seam: the library carries every panel as
/// a name and a view, and a title is presentation.
fn tab_of(panel: &Arc<dyn BasePanelView>, cx: &App) -> (SharedString, Option<gpui::Rgba>) {
    match panel.view().downcast::<WorkbenchPanel>() {
        Ok(panel) => panel.read(cx).tab(cx),
        Err(_) => (SharedString::from(panel.panel_name(cx)), None),
    }
}

/// The label that follows the pointer while a tab is dragged. The library's drag payload renders
/// nothing, because a preview is appearance.
struct DragLabel {
    title: SharedString,
}

impl gpui::Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .text_size(px(12.5))
            .bg(theme::surface())
            .text_color(theme::text())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::accent())
            .child(self.title.clone())
    }
}

/// Everything Ubiq draws around a panel. One value implements all three renderer traits, because
/// they share a palette and nothing else.
#[derive(Clone, Default)]
pub struct Skin {
    /// The region a resize drag is sizing, captured on mouse down. A resize follows the pointer
    /// anywhere in the window rather than only over the strip, so the listener that tracks it sits
    /// on the area's frame — which is not handed a region.
    resizing: Rc<RefCell<Option<DockContext>>>,
}

impl Skin {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// The strip on a region's inner edge that resizes it.
    fn resize_strip(&self, dock: &DockContext) -> impl IntoElement {
        let placement = dock.placement();
        let dock = dock.clone();
        let resizing = self.resizing.clone();

        div()
            .absolute()
            .flex()
            .items_center()
            .justify_center()
            .map(|this| match placement {
                DockPlacement::Left => this
                    .top_0()
                    .right_0()
                    .h_full()
                    .w(px(RESIZE_STRIP))
                    .cursor_col_resize(),
                DockPlacement::Bottom => this
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(RESIZE_STRIP))
                    .cursor_row_resize(),
                _ => this
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(px(RESIZE_STRIP))
                    .cursor_col_resize(),
            })
            .child(div().bg(theme::border()).map(|line| match placement {
                DockPlacement::Bottom => line.h(px(1.)).w_full(),
                _ => line.w(px(1.)).h_full(),
            }))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                *resizing.borrow_mut() = Some(dock.clone());
            })
    }
}

impl DockAreaRenderer for Skin {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        let dragging = self.resizing.clone();
        let finished = self.resizing.clone();

        div()
            .id("ubiq-dock")
            .size_full()
            .flex()
            .flex_row()
            .min_h(px(0.))
            .overflow_hidden()
            .bg(theme::app_bg())
            .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                // Cloned out before the call, so the borrow is released before the resize reaches
                // back into another frame reading this cell.
                let dock = dragging.borrow().clone();
                let Some(dock) = dock else { return };
                dock.resize_to(event.position, window, cx);
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, _| {
                finished.borrow_mut().take();
            })
    }

    fn center_frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("ubiq-dock-centre")
            .flex()
            .flex_1()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: gpui_component::dock::NodeId,
        _: gpui::Axis,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("ubiq-dock-split", node.as_u64()))
            .size_full()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        // A closed region takes no space at all. The titlebar's switches are what bring it back.
        if !dock.is_open() {
            return div().into_any_element();
        }

        div()
            .flex()
            .flex_none()
            .relative()
            .overflow_hidden()
            .map(|this| match dock.placement() {
                DockPlacement::Bottom => this.w_full().h(dock.size()).flex_col(),
                _ => this.h_full().w(dock.size()).flex_row(),
            })
            .child(content)
            .child(self.resize_strip(dock))
            .into_any_element()
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(self.clone())
    }

    fn tiles_renderer(&self) -> Rc<dyn TilesRenderer> {
        Rc::new(self.clone())
    }
}

impl TabGroupRenderer for Skin {
    fn frame(&self, _: &TabGroupContext, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("ubiq-tab-group")
            .size_full()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .bg(theme::pane_bg())
    }

    fn content_frame(&self, _: &TabGroupContext, _: &mut Window, _: &mut App) -> Stateful<Div> {
        // Relative, because the drop indicator is positioned against it.
        div()
            .id("ubiq-tab-content")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .bg(theme::app_bg())
    }

    /// The group's tabs: one per panel, the displayed one marked on its bottom edge, each carrying
    /// the panel's own dot and — where the panel allows it — a close.
    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let tabs: Vec<_> = group
            .panels()
            .iter()
            .enumerate()
            // A hidden panel keeps its place in the tree and its tab slot; the skin is what leaves
            // it undrawn.
            .filter(|(_, panel)| panel.visible(cx))
            .map(|(ix, panel)| {
                let active = ix == group.active_ix();
                let (title, dot) = tab_of(panel, cx);
                let panel_id = panel.panel_id(cx);
                let closable = group.is_closable() && panel.closable(cx);

                let mut tab = div()
                    .id(("ubiq-tab", ix))
                    .h(px(TAB_BAR))
                    .px_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .border_b_2()
                    .border_color(if active {
                        theme::accent()
                    } else {
                        theme::border()
                    })
                    .text_size(px(12.5))
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::text_muted()
                    })
                    .cursor_pointer()
                    .hover(|this| this.text_color(theme::text()));

                if active {
                    tab = tab.bg(theme::app_bg());
                }

                if let Some(colour) = dot {
                    tab = tab.child(div().size(px(7.)).flex_none().rounded_full().bg(colour));
                }

                tab = tab.child(title.clone());

                if closable {
                    let group = group.clone();
                    tab = tab.child(
                        div()
                            .id(("ubiq-tab-close", ix))
                            .size(px(16.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|this| this.bg(theme::hover()))
                            .child(
                                Icon::new(IconName::Close)
                                    .with_size(Size::XSmall)
                                    .text_color(theme::text_faint()),
                            )
                            .on_click(move |_, window, cx| group.close(panel_id, window, cx)),
                    );
                }

                let select = group.clone();
                tab.on_click(move |_, window, cx| select.select_tab(ix, window, cx))
                    // The drag is the library's: dropping this tab re-parents a panel *id*, so the
                    // entity behind it is never rebuilt and a dragged terminal keeps its stream.
                    .when_some(group.drag_panel(ix, cx), |this, drag| {
                        let title = title.clone();
                        this.on_drag(drag, move |_, _, _, cx| {
                            cx.new(|_| DragLabel {
                                title: title.clone(),
                            })
                        })
                    })
            })
            .collect();

        // Zoom is the group's, not a panel's: it fills the region with whichever panel the group
        // is displaying, and the control has to stay on screen to give the region back.
        let zoomable = group
            .active_panel()
            .is_some_and(|panel| panel.zoomable(cx) || group.is_zoomed());
        let zoom = group.clone();

        div()
            .h(px(TAB_BAR))
            .flex()
            .flex_none()
            .items_center()
            .bg(theme::pane_bg())
            .border_b_1()
            .border_color(theme::border())
            .font_weight(FontWeight::NORMAL)
            .overflow_hidden()
            .children(tabs)
            .child(div().flex_1().min_w(px(0.)))
            .when(zoomable, |this| {
                this.child(
                    div()
                        .id("ubiq-tab-zoom")
                        .mr_2()
                        .size(px(20.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|this| this.bg(theme::hover()))
                        .child(
                            Icon::new(if group.is_zoomed() {
                                IconName::Minimize
                            } else {
                                IconName::Maximize
                            })
                            .with_size(Size::XSmall)
                            .text_color(theme::text_faint()),
                        )
                        .on_click(move |_, window, cx| zoom.toggle_zoom(window, cx)),
                )
            })
            .into_any_element()
    }

    /// The panel fills its group. `D18`'s edge has to land on the group's own boundary, so nothing
    /// is inset here.
    fn render_active_panel(
        &self,
        panel: AnyView,
        _: &TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .child(panel)
            .into_any_element()
    }

    /// Where the drop would land. The library resolved it; painting it is all that is left.
    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        _: &mut Window,
        _: &mut App,
    ) -> Option<AnyElement> {
        let to = indicator.to();
        Some(
            div()
                .absolute()
                .left(to.origin().x)
                .top(to.origin().y)
                .w(to.size().width)
                .h(to.size().height)
                .bg(theme::accent_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent())
                .into_any_element(),
        )
    }

    /// A group whose every panel is hidden. It keeps its shape rather than collapsing mid-drag.
    fn render_empty(&self, _: &TabGroupContext, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        Some(div().flex_1().bg(theme::app_bg()).into_any_element())
    }
}

/// Ubiq builds no tiles canvas — the free-floating one the library offers is a backlog row — but a
/// dock renderer must still name a tiles renderer, because the library builds one for any canvas a
/// layout happens to hold.
impl TilesRenderer for Skin {
    fn render_drag_bar(&self, _: &TileContext, _: &mut Window, _: &mut App) -> AnyElement {
        div().into_any_element()
    }
}
