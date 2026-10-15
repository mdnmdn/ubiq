//! **Text selection for the rendered view** — one block at a time.
//!
//! A drag that starts in a paragraph, a heading, a code fence or a table cell selects inside that
//! one text element and stops at its edges; it never spans two blocks. That is the whole scope:
//! the rows are separate elements in a virtualised `list()`, so a selection across them would need
//! a document-wide text model this view does not have. `Cmd+C` copies the selected text.
//!
//! The state lives here, in a handle the view owns and every row is handed through
//! [`super::blocks::Metrics`], because the rows are rebuilt every frame and nothing in an element
//! survives one. A press records *where* (a [`Key`]: the block's source offset and a sub-index for
//! a table cell), *what* (the element's text, so a stale selection over edited text is dropped
//! rather than indexed), and a [`Locate`] closure that turns a pointer position into a byte offset
//! in that element — which is how the drag keeps working once the pointer has left it.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, ClipboardItem, Hsla, InteractiveElement, MouseButton, Pixels, Point, SharedString,
    StyledText, TextRun,
};

use crate::theme;

/// What a selection belongs to: a block's source offset, and which text element inside it.
pub type Key = (usize, usize);

/// A pointer position, as a byte offset into one element's text.
pub type Locate = Rc<dyn Fn(Point<Pixels>) -> usize>;

/// Asks the owning view to repaint.
pub type Notify = Rc<dyn Fn(&mut App)>;

#[derive(Default)]
struct Inner {
    key: Option<Key>,
    text: SharedString,
    anchor: usize,
    head: usize,
    dragging: bool,
    locate: Option<Locate>,
    notify: Option<Notify>,
}

/// `runs` with `range` (bytes of the text they cover) given `background`, splitting the runs that
/// straddle its edges. The run lengths still sum to what they did.
fn mark(runs: Vec<TextRun>, range: Range<usize>, background: Hsla) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        let piece = |from: usize, to: usize, marked: bool| {
            (to > from).then(|| TextRun {
                len: to - from,
                background_color: if marked {
                    Some(background)
                } else {
                    run.background_color
                },
                ..run.clone()
            })
        };
        let inside_from = range.start.clamp(start, end);
        let inside_to = range.end.clamp(start, end);
        if inside_to <= inside_from {
            out.extend(piece(start, end, false));
        } else {
            out.extend(piece(start, inside_from, false));
            out.extend(piece(inside_from, inside_to, true));
            out.extend(piece(inside_to, end, false));
        }
        start = end;
    }
    out
}

/// The view's one selection. Cheap to clone; every clone is the same selection.
#[derive(Clone, Default)]
pub struct Selection(Rc<RefCell<Inner>>);

impl Selection {
    /// How the view is told to repaint when the selection moves.
    pub fn set_notify(&self, notify: Notify) {
        self.0.borrow_mut().notify = Some(notify);
    }

    /// Drop the selection. Whether there was one to drop.
    pub fn clear(&self) -> bool {
        let mut inner = self.0.borrow_mut();
        let had = inner.key.is_some();
        inner.key = None;
        inner.dragging = false;
        inner.locate = None;
        had
    }

    /// A press in element `key` at byte `at`: a new, empty selection, and a drag in flight.
    pub fn press(&self, key: Key, text: SharedString, at: usize, locate: Locate, cx: &mut App) {
        {
            let mut inner = self.0.borrow_mut();
            inner.key = Some(key);
            inner.text = text;
            inner.anchor = at;
            inner.head = at;
            inner.dragging = true;
            inner.locate = Some(locate);
        }
        self.repaint(cx);
    }

    /// The pointer moved with the button down. Whether a drag was in flight.
    pub fn drag(&self, position: Point<Pixels>, cx: &mut App) -> bool {
        let moved = {
            let mut inner = self.0.borrow_mut();
            if !inner.dragging {
                return false;
            }
            let Some(locate) = inner.locate.clone() else {
                return false;
            };
            let head = locate(position);
            let moved = head != inner.head;
            inner.head = head;
            moved
        };
        if moved {
            self.repaint(cx);
        }
        true
    }

    /// Re-point the drag at `locate`, when element `key` is the one being selected: a frame that
    /// scrolled or reflowed moved the element, and the closure from the press knows the old place.
    pub fn refresh(&self, key: Key, locate: Locate) {
        let mut inner = self.0.borrow_mut();
        if inner.key == Some(key) && inner.dragging {
            inner.locate = Some(locate);
        }
    }

    /// The button came up.
    pub fn release(&self) {
        self.0.borrow_mut().dragging = false;
    }

    /// The selected span of element `key`, if that is where the selection is and `text` is still
    /// what it was selected in.
    pub fn range(&self, key: Key, text: &str) -> Option<Range<usize>> {
        let inner = self.0.borrow();
        if inner.key != Some(key) || inner.text.as_ref() != text {
            return None;
        }
        let (from, to) = (inner.anchor.min(inner.head), inner.anchor.max(inner.head));
        let to = to.min(text.len());
        (from < to && text.is_char_boundary(from) && text.is_char_boundary(to)).then_some(from..to)
    }

    /// The selected text, when there is any.
    pub fn selected(&self) -> Option<String> {
        let inner = self.0.borrow();
        inner.key?;
        let text = inner.text.as_ref();
        let (from, to) = (inner.anchor.min(inner.head), inner.anchor.max(inner.head));
        let to = to.min(text.len());
        (from < to && text.is_char_boundary(from) && text.is_char_boundary(to))
            .then(|| text[from..to].to_string())
    }

    /// Put the selected text on the clipboard. Whether there was any.
    pub fn copy(&self, cx: &mut App) -> bool {
        match self.selected() {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            None => false,
        }
    }

    fn repaint(&self, cx: &mut App) {
        let notify = self.0.borrow().notify.clone();
        if let Some(notify) = notify {
            notify(cx);
        }
    }

    /// The element for `text` shaped by `runs`, with this selection painted over it when the
    /// selection is in element `key`.
    ///
    /// The selection is marked into the runs themselves: `StyledText::with_highlights` is ignored
    /// once an element has explicit runs (and asserts in a debug build), and every caller here has
    /// them — the mono face, a cell's marks. Pass the result to [`Selection::bind`] too, for its
    /// layout.
    pub fn paint(&self, text: SharedString, runs: Vec<TextRun>, key: Key) -> StyledText {
        let runs = match self.range(key, &text) {
            Some(range) => mark(runs, range, theme::accent_selection().into()),
            None => runs,
        };
        StyledText::new(text).with_runs(runs)
    }

    /// Make `element` a place a selection can start: a left press anywhere in it records the
    /// byte offset under the pointer in `styled`'s text.
    ///
    /// Takes the layout before the `StyledText` is consumed into an element; the layout is shared
    /// state the element fills in as it paints, so it answers once the frame has drawn.
    pub fn bind<E: InteractiveElement>(
        &self,
        element: E,
        styled: &StyledText,
        key: Key,
        text: SharedString,
    ) -> E {
        let layout = styled.layout().clone();
        let selection = self.clone();
        element.on_mouse_down(MouseButton::Left, move |event, _window, cx| {
            let locate: Locate = {
                let layout = layout.clone();
                Rc::new(move |position| match layout.index_for_position(position) {
                    Ok(at) | Err(at) => at,
                })
            };
            let at = locate(event.position);
            selection.press(key, text.clone(), at, locate, cx);
        })
    }
}
