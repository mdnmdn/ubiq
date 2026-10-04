//! The markdown view entity over a shared buffer: an edit to the buffer reaches the block table one
//! debounce later, and a block commit writes the buffer, which the same debounce then reparses.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_component::input::EditorState;
use ubiq::state::document::RowDecor;
use ubiq::ui::mdview::MdViewConfig;
use ubiq::ui::mdview::annotation::{RowActions, RowMenu};
use ubiq::ui::mdview::blockedit;
use ubiq::ui::mdview::events::MdViewEvent;
use ubiq::ui::mdview::view::{MdView, REPARSE_DEBOUNCE};
use ubiq_proto::plan::{AnnotationMark, HighlightColour};

struct Fixture {
    buffer: Entity<EditorState>,
    view: Entity<MdView>,
    events: Rc<RefCell<Vec<MdViewEvent>>>,
    _events: gpui::Subscription,
}

fn open<'a>(cx: &'a mut TestAppContext, source: &str) -> (Fixture, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
    });
    let cx = cx.add_empty_window();
    let source = source.to_string();
    let events: Rc<RefCell<Vec<MdViewEvent>>> = Rc::default();
    let sink = events.clone();
    let (buffer, view, subscription) = cx.update(|window, cx| {
        let buffer = cx.new(|cx| EditorState::new(window, cx).default_value(source));
        let view = cx.new(|cx| MdView::new(buffer.clone(), MdViewConfig::default(), cx));
        let subscription = cx.subscribe(&view, move |_, event: &MdViewEvent, _| {
            sink.borrow_mut().push(event.clone());
        });
        (buffer, view, subscription)
    });
    (
        Fixture {
            buffer,
            view,
            events,
            _events: subscription,
        },
        cx,
    )
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor()
        .advance_clock(REPARSE_DEBOUNCE + Duration::from_millis(20));
    cx.run_until_parked();
}

fn blocks(fixture: &Fixture, cx: &mut VisualTestContext) -> usize {
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.list().item_count(), view.doc().blocks.len());
        view.doc().blocks.len()
    })
}

#[gpui::test]
fn the_block_table_follows_the_buffer(cx: &mut TestAppContext) {
    let (fixture, cx) = open(cx, "# Title\n\nalpha\n");
    assert_eq!(blocks(&fixture, cx), 2);

    fixture.buffer.update_in(cx, |state, window, cx| {
        state.replace_all("# Title\n\nalpha\n\nbeta\n\n- one\n- two\n", window, cx)
    });
    // Nothing moves inside the debounce window.
    assert_eq!(blocks(&fixture, cx), 2);

    settle(cx);
    assert_eq!(blocks(&fixture, cx), 4);
    assert!(
        fixture
            .events
            .borrow()
            .contains(&MdViewEvent::DocumentChanged { blocks: 4 })
    );
}

#[gpui::test]
fn a_block_commit_writes_the_buffer_and_reparses(cx: &mut TestAppContext) {
    let (fixture, cx) = open(cx, "alpha\n\nbeta\n\ngamma\n");
    fixture.view.update_in(cx, |view, window, cx| {
        view.set_editable(true, window, cx);
        blockedit::open(view, 1, window, cx);
    });
    let state = fixture
        .view
        .read_with(cx, |view, _| view.editing().map(|edit| edit.state.clone()))
        .expect("the editor opened");
    state.update_in(cx, |state, window, cx| {
        state.set_value("beta one\n\nbeta two", window, cx)
    });
    let committed = fixture.view.update_in(cx, blockedit::commit);
    assert!(committed);

    let text = fixture
        .buffer
        .read_with(cx, |state, _| state.value().to_string());
    assert_eq!(text, "alpha\n\nbeta one\n\nbeta two\n\ngamma\n");
    settle(cx);
    assert_eq!(blocks(&fixture, cx), 4);
    assert!(fixture.events.borrow().iter().any(|event| matches!(
        event,
        MdViewEvent::BlockEdited { blocks, .. } if *blocks == (1..2)
    )));
}

// ── The annotation layer ─────────────────────────────────────────────

fn decor(row: usize, open: usize, resolved: usize) -> RowDecor {
    RowDecor {
        row,
        open,
        resolved,
        marks: Vec::new(),
        highlight: None,
        focused: false,
    }
}

/// Pushed decor is what the margins read: a count on every row with threads while annotating, the
/// stack on the selected row only, the highlight dot in every mode.
#[gpui::test]
fn pushed_decor_shows_on_its_row(cx: &mut TestAppContext) {
    let (fixture, cx) = open(cx, "alpha\n\nbeta\n\ngamma\n");
    fixture.view.update(cx, |view, cx| {
        let mut busy = decor(1, 2, 1);
        busy.marks = vec![AnnotationMark::Agent];
        let mut painted = decor(2, 0, 0);
        painted.highlight = Some(HighlightColour::Blue);
        // Out of order on purpose: the view sorts.
        view.set_decor(vec![painted, busy], cx);
    });

    fixture.view.read_with(cx, |view, _| {
        // Not annotating: only the dot.
        assert!(view.row_actions(1).marker.is_none());
        assert_eq!(view.row_actions(2).dot, Some(HighlightColour::Blue));
    });

    fixture.view.update(cx, |view, cx| {
        view.set_annotating(true, cx);
        view.select_block(1, cx);
    });
    fixture.view.read_with(cx, |view, _| {
        let busy = view.row_actions(1);
        let marker = busy.marker.expect("a row with threads has a count");
        assert_eq!(marker.threads, 3);
        assert!(marker.for_agent && !marker.all_resolved);
        assert!(busy.add_thread && busy.mark && busy.highlight_menu && busy.selected_bar);
        assert_eq!(busy.resolve, Some(true));
        assert!(!busy.edit, "read-only: no ✎");

        let painted = view.row_actions(2);
        assert_eq!(painted.dot, Some(HighlightColour::Blue));
        assert!(painted.marker.is_none() && !painted.add_thread);
        assert_eq!(view.row_actions(0), RowActions::default());
    });
}

/// Every margin control is an intent: it emits, closes the open menu, and leaves the decor alone.
#[gpui::test]
fn the_margin_controls_emit_intents(cx: &mut TestAppContext) {
    let (fixture, cx) = open(cx, "alpha\n\nbeta\n\ngamma\n");
    fixture.view.update(cx, |view, cx| {
        view.set_annotating(true, cx);
        view.set_decor(vec![decor(0, 1, 0), decor(1, 0, 2)], cx);
        view.add_thread(2, cx);
        view.threads_clicked(1, cx);

        view.open_row_menu(RowMenu::Mark(0), cx);
        assert_eq!(view.open_menu(), Some(RowMenu::Mark(0)));
        view.request_mark(0, AnnotationMark::Todo, cx);
        assert_eq!(view.open_menu(), None, "a pick closes the menu");

        view.request_resolve(0, cx); // open thread: resolve
        view.request_resolve(1, cx); // all resolved: reopen
        view.request_resolve(2, cx); // no thread: nothing

        view.open_row_menu(RowMenu::Highlight(0), cx);
        view.open_row_menu(RowMenu::Mark(0), cx);
        assert_eq!(
            view.open_menu(),
            Some(RowMenu::Mark(0)),
            "one menu at a time"
        );
        view.request_highlight(0, Some(HighlightColour::Purple), cx);
        view.request_highlight(0, None, cx);
        assert_eq!(view.decor(), &[decor(0, 1, 0), decor(1, 0, 2)]);
    });

    let intents: Vec<MdViewEvent> = fixture
        .events
        .borrow()
        .iter()
        .filter(|event| !matches!(event, MdViewEvent::BlockSelected { .. }))
        .cloned()
        .collect();
    assert_eq!(
        intents,
        vec![
            MdViewEvent::AddThread { row: 2 },
            MdViewEvent::ThreadsClicked { row: 1 },
            MdViewEvent::MarkRequested {
                row: 0,
                mark: AnnotationMark::Todo
            },
            MdViewEvent::ResolveRequested {
                row: 0,
                resolved: true
            },
            MdViewEvent::ResolveRequested {
                row: 1,
                resolved: false
            },
            MdViewEvent::HighlightRequested {
                row: 0,
                colour: Some(HighlightColour::Purple)
            },
            MdViewEvent::HighlightRequested {
                row: 0,
                colour: None
            },
        ]
    );
}

/// Drawn: the count and the stack sit in the far margin, the dot in the left one, no row changes
/// height for carrying them, and a click on a control is its intent.
#[gpui::test]
fn the_decor_draws_in_the_margins_without_moving_a_row(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let buffer = cx.new(|cx| {
            EditorState::new(window, cx).default_value("# Title\n\nalpha beta\n\ngamma\n")
        });
        MdView::new(buffer, MdViewConfig::default(), cx)
    });
    let events: Rc<RefCell<Vec<MdViewEvent>>> = Rc::default();
    let sink = events.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &MdViewEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
    });
    view.update(cx, |view, cx| view.set_annotating(true, cx));
    cx.run_until_parked();
    let heights = |view: &Entity<MdView>, cx: &mut VisualTestContext| -> Vec<f32> {
        view.read_with(cx, |view, _| {
            (0..view.doc().blocks.len())
                .map(|ix| {
                    view.list()
                        .bounds_for_item(ix)
                        .map_or(-1.0, |b| f32::from(b.size.height))
                })
                .collect()
        })
    };
    let before = heights(&view, cx);

    view.update(cx, |view, cx| {
        let mut row = decor(1, 2, 0);
        row.highlight = Some(HighlightColour::Yellow);
        view.set_decor(vec![row], cx);
        view.select_block(1, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        heights(&view, cx),
        before,
        "decor never changes a row's height"
    );

    let count = cx.debug_bounds("md-threads-1").expect("the count draws");
    let add = cx.debug_bounds("md-new-thread-1").expect("the stack draws");
    let dot = cx.debug_bounds("md-highlight-1").expect("the dot draws");
    let pane = cx.update(|window, _| f32::from(window.viewport_size().width));
    assert!(
        f32::from(count.origin.x) > pane / 2.0,
        "the count is in the far margin"
    );
    assert!(
        f32::from(dot.origin.x) < pane / 2.0,
        "the dot is in the left margin"
    );

    cx.simulate_click(count.center(), gpui::Modifiers::none());
    cx.simulate_click(add.center(), gpui::Modifiers::none());
    let intents: Vec<MdViewEvent> = events
        .borrow()
        .iter()
        .filter(|event| !matches!(event, MdViewEvent::BlockSelected { .. }))
        .cloned()
        .collect();
    assert_eq!(
        intents,
        vec![
            MdViewEvent::ThreadsClicked { row: 1 },
            MdViewEvent::AddThread { row: 1 },
        ]
    );
}
