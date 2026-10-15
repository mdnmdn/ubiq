//! **The expand affordance and the full-window dialog it opens** for the two blocks that outgrow
//! the reading column: a code fence and a table.
//!
//! A small button sits in the block's top-right corner and shows on hover. It hands the view a
//! [`Zoom`] — a copy of what the block draws — and the view paints [`overlay`] over everything
//! until Esc, the × or a press on the scrim closes it. The dialog's body scrolls both ways and its
//! text is selectable like the page's; its header carries a Copy button for the whole content.
//!
//! The dialog is the view's own rather than a layer in `app/shell.rs`'s Escape ladder: the view
//! holds the keyboard focus while it is up (`MdView::open_zoom`), and handles its own Esc.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::IconName;
use ubiq_md::{Block, BlockKind, Cell, ColumnAlign};

use super::inline::{self, Base};
use crate::theme::{self, Family, Role};
use crate::ui::kit;

/// What the dialog shows: a copy of one block, enough to draw it again.
#[derive(Clone)]
pub enum Zoom {
    Code {
        language: Option<String>,
        body: String,
    },
    Table {
        alignments: Vec<ColumnAlign>,
        head: Vec<Cell>,
        rows: Vec<Vec<Cell>>,
    },
}

/// What a click on a block's expand button does. The view supplies it.
///
/// It is given the block's source offset, not a copy of the block: the view finds the block in its
/// document and builds the [`Zoom`] on the click, so nothing is cloned per frame.
pub type ZoomHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

impl Zoom {
    /// The dialog's content for the block at source offset `at`, searching nested blocks too.
    pub fn find(blocks: &[Block], at: usize) -> Option<Zoom> {
        for block in blocks {
            let nested: Vec<&[Block]> = match &block.kind {
                BlockKind::Code { .. } | BlockKind::Table { .. } if block.range.start == at => {
                    return match &block.kind {
                        BlockKind::Code { language, body, .. } => Some(Zoom::Code {
                            language: language.clone(),
                            body: body.clone(),
                        }),
                        BlockKind::Table {
                            alignments,
                            head,
                            rows,
                        } => Some(Zoom::Table {
                            alignments: alignments.clone(),
                            head: head.clone(),
                            rows: rows.clone(),
                        }),
                        _ => None,
                    };
                }
                BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                    vec![blocks]
                }
                BlockKind::List { items, .. } => {
                    items.iter().map(|item| item.blocks.as_slice()).collect()
                }
                _ => Vec::new(),
            };
            if let Some(found) = nested.into_iter().find_map(|inner| Zoom::find(inner, at)) {
                return Some(found);
            }
        }
        None
    }

    /// The dialog's title.
    pub fn title(&self) -> String {
        match self {
            Zoom::Code {
                language: Some(language),
                ..
            } => format!("Code \u{b7} {language}"),
            Zoom::Code { .. } => "Code".to_string(),
            Zoom::Table { .. } => "Table".to_string(),
        }
    }

    /// The whole content as plain text — what Copy puts on the clipboard. A table is tab- and
    /// newline-separated, which is what a spreadsheet pastes as a grid.
    pub fn plain(&self) -> String {
        match self {
            Zoom::Code { body, .. } => body.trim_end_matches('\n').to_string(),
            Zoom::Table { head, rows, .. } => {
                let line = |cells: &Vec<Cell>| {
                    cells
                        .iter()
                        .map(|cell| {
                            inline::runs(&cell.spans, &Base::body())
                                .0
                                .replace(['\n', '\t'], " ")
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                };
                std::iter::once(line(head))
                    .chain(rows.iter().map(line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

/// `container` with the expand button in its top-right corner, shown while the pointer is over the
/// block. Unchanged when there is no handler to take the click.
///
/// `at` is the block's source offset, which the click hands to `on_zoom`.
///
/// `content` is the block's own children, taken here rather than added by the caller afterwards:
/// siblings paint in order, so the button has to be the last child or the block's own backgrounds
/// (a table's header row) are drawn over it.
pub fn expandable(
    container: Div,
    at: usize,
    on_zoom: Option<&ZoomHandler>,
    content: impl IntoIterator<Item = AnyElement>,
) -> Div {
    let container = container.children(content);
    let Some(on_zoom) = on_zoom else {
        return container;
    };
    let on_zoom = on_zoom.clone();
    let group = format!("md-zoom-{at}");

    let button = kit::icon_button_tip(
        ("md-zoom", at),
        IconName::Maximize,
        "Expand",
        true,
        move |_, window, cx| on_zoom(at, window, cx),
    )
    .absolute()
    .top_1()
    .right_1()
    .size(px(24.))
    .bg(theme::surface_raised())
    .border_1()
    .border_color(theme::border())
    .opacity(0.)
    .group_hover(group.clone(), |this| this.opacity(1.))
    // A press on the button is the button's: it must not start a text selection under it or
    // select the row.
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    container.relative().group(group).child(button)
}

/// The dialog: near the window's size, `body` scrolling both ways inside it.
pub fn overlay(
    zoom: &Zoom,
    body: AnyElement,
    on_copy: impl Fn(&mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
) -> AnyElement {
    let viewport = window.viewport_size();
    let scroller = div()
        .id("md-zoom-scroll")
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_scroll()
        .bg(theme::markdown().code_bg)
        .border_1()
        .border_color(theme::border())
        .child(body)
        .into_any_element();
    let copy = kit::icon_button_tip(
        "md-zoom-copy",
        IconName::Copy,
        "Copy all",
        true,
        move |_, window, cx| on_copy(window, cx),
    );
    let hint = div()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(theme::text_faint())
        .child("Select text and press Cmd+C to copy a part \u{b7} Esc closes")
        .into_any_element();

    kit::modal_sized(
        "md-zoom-dialog",
        theme::accent(),
        f32::from(viewport.width) * 0.94,
        Some(f32::from(viewport.height) * theme::MODAL_MAX_HEIGHT),
        &zoom.title(),
        Some(copy.into_any_element()),
        scroller,
        hint,
        on_close,
        window,
    )
}
