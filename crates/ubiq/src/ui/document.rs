//! The annotated-document surface: an `MdView` with its annotation layer on, and the thread rail
//! beside it.
//!
//! **One surface, two frames.** `crate::ui::plan` hosts it in the plan dialog and
//! `crate::ui::viewer` hosts it as a markdown tab's fourth layout (`ViewLayout::Annotation`), and
//! nothing below knows which — it reads a `DocumentEditor`, whose `md` is the dialog's own view or
//! the tab's, and whose `presentation` is the only other thing that differs.
//!
//! **The page is the view's; the threads are the rail's.** The view draws each row's margin — the
//! count marker, the `⚑ ✓ + ● ✎` stack under the selected row, the highlight dot — from the decor
//! `AppState` pushes, and raises every press there as an intent `crate::app::plan` acts on. A
//! thread's notes appear only here, never inline in the page.
//!
//! **The rail is a chat-like list in document order** (`state::document::ordered`): every thread
//! collapsed to its block's first line, its marks and its first comment, except the focused one,
//! which is expanded — every comment, its mark chips, resolve, and the reply field. The focus
//! follows the page's scroll to the first open thread on screen, and a click on a thread focuses
//! it and brings its block into view. A fresh thread is written in the composer at the rail's foot,
//! with its mark chips and the "@agent" toggle.

use gpui::{
    AnyElement, App, Context, CursorStyle, DragMoveEvent, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, list, px,
};
use gpui::AppContext as _;
use gpui_component::input::Textarea;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::document::{
    AnnotationsBody, ComposerTarget, DocumentBody, DocumentEditor, Notice,
};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid;
use crate::ui::eid2;
use crate::state::status::{Lifecycle, agent_status};
use crate::state::workbench::MenuId;
use crate::ui::kit::{
    Picker, PickerStyle, check_box, choice_pill, ghost_button, icon_button, primary_button, slab, toggle_pill,
};
use crate::ui::teams::status::{card_colour, status_mark};
use crate::ui::{handler, indexed};
use ubiq_proto::ids::AnnotationId;
use ubiq_proto::work::AgentId;
use crate::ui::mdview::annotation::{MARKS, mark_label};
use crate::ui::mdview::view::MdView;
use ubiq_proto::plan::{Annotation, AnnotationMark, AnnotationState};
use ubiq_proto::work::{Comment, CommentAuthor};

/// The rail is a fixed column: a thread is short, and a wider one would only narrow the document.
pub const RAIL_WIDTH: f32 = 320.0;
/// The narrowest and widest the rail is dragged to; the page keeps at least `PAGE_MIN`.
const RAIL_MIN: f32 = 240.0;
const RAIL_MAX: f32 = 640.0;
const PAGE_MIN: f32 = 280.0;

/// The rail's edge, as a drag payload: its own type, so no other drop target answers it.
#[derive(Clone)]
struct RailEdge;

/// What follows the pointer while the edge is dragged: nothing, the rail itself moves.
struct NoGhost;

impl gpui::Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
/// The chrome strip above the rail, the file viewer's own header height.
const HEADER: f32 = 32.0;
/// How far past the viewport the rail keeps threads rendered, so a scroll has something already
/// laid out to reveal.
pub const PLAN_OVERDRAW: f32 = 600.0;
/// How much of a block's first line, or a comment's, a collapsed thread shows.
const EXCERPT_CHARS: usize = 72;

/// The whole surface below whatever chrome its frame drew: the notices, then the page and the rail.
pub fn surface(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let page = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(page(app, doc, cx));
    let rail_col = div()
        .flex_none()
        .w(px(app.workbench.doc_rail_width.clamp(RAIL_MIN, RAIL_MAX)))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .relative()
        .border_l_1()
        .border_color(theme::border())
        .child(rail(app, doc, cx))
        .child(
            div()
                .id(eid("doc-rail-edge", &doc.surface_key()))
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-3.))
                .w(px(6.))
                .cursor(CursorStyle::ResizeLeftRight)
                .hover(|this| this.bg(theme::accent_soft()))
                .on_drag(RailEdge, |_, _, _, cx: &mut App| cx.new(|_| NoGhost)),
        );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<RailEdge>, _, cx| {
                let right = f32::from(event.bounds.right());
                let room = f32::from(event.bounds.size.width) - PAGE_MIN;
                let width = (right - f32::from(event.event.position.x))
                    .clamp(RAIL_MIN, RAIL_MAX.min(room.max(RAIL_MIN)));
                this.workbench.doc_rail_width = width;
                cx.notify();
            }),
        )
        .children(doc.notice.as_ref().map(banner))
        .children(state_banner(doc, cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .border_t_1()
                .border_color(theme::border())
                .child(page)
                .child(rail_col),
        )
        .into_any_element()
}

/// The page: the document's view, with the window's diagram cache published for its fences. The
/// dialog waits for the host's body; a tab's view is over the tab's own buffer and is drawn as
/// soon as the tab has one.
fn page(app: &AppState, doc: &DocumentEditor, cx: &App) -> AnyElement {
    if doc.is_modal() {
        match &doc.body {
            DocumentBody::Loading => return note("Reading\u{2026}", theme::text_faint()),
            DocumentBody::Failed(reason) => return note(reason.clone(), theme::danger()),
            DocumentBody::Loaded(_) => {}
        }
    }
    doc.md.read(cx).publish_fences(app);
    doc.md.clone().into_any_element()
}

/// The header's heading navigator over a markdown view: how many headings, and a press opens the
/// view's own jump popover. Shared by the dialog's chrome and a markdown tab's header.
pub fn heading_control(
    id: impl Into<gpui::ElementId>,
    md: &Entity<MdView>,
    cx: &App,
) -> AnyElement {
    let headings = md.read(cx).heading_count();
    let md = md.clone();
    div()
        .id(id)
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .cursor_pointer()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_muted())
        .hover(|this| this.bg(theme::hover()).text_color(theme::text()))
        .child(format!("{headings} headings"))
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .tooltip(|window, cx| Tooltip::new("Go to heading").build(window, cx))
        .on_click(move |_, window, cx| {
            md.update(cx, |md, cx| md.open_navigator(window, cx));
        })
        .into_any_element()
}

/// A markdown view's read-only / editable switch: on, a double-click on a block (or the row's `✎`)
/// opens the block editor in its place.
pub fn edit_chip(id: impl Into<gpui::ElementId>, md: &Entity<MdView>, cx: &App) -> AnyElement {
    let on = md.read(cx).editable();
    let md = md.clone();
    toggle_pill(id, "Edit", theme::accent(), on, move |_, window, cx| {
        md.update(cx, |md, cx| md.set_editable(!on, window, cx))
    })
    .into_any_element()
}

/// The thread rail: the header, the list, and the composer at the foot.
fn rail(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let key = doc.surface_key();
    let body = match &doc.annotations {
        AnnotationsBody::Loading => note("Reading annotations\u{2026}", theme::text_faint()),
        AnnotationsBody::Failed(reason) => note(reason.clone(), theme::danger()),
        AnnotationsBody::Loaded { .. } => thread_list(app, doc, cx),
    };
    let resolved = doc
        .annotations
        .annotations()
        .iter()
        .filter(|annotation| !annotation.is_open())
        .count();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex_none()
                .h(px(HEADER))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_muted())
                .border_b_1()
                .border_color(theme::border())
                .child("Threads")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(check_box(
                            eid("plan-track-updates", &key),
                            doc.track_updates,
                            cx.listener(|this, _, _, cx| this.toggle_track_updates(cx)),
                        ))
                        .child("Track changes"),
                )
                .child(div().flex_1().min_w(px(0.)))
                .children((resolved > 0).then(|| {
                    choice_pill(
                        eid("plan-show-resolved", &key),
                        format!("{resolved} resolved"),
                        doc.show_resolved,
                        cx.listener(|this, _, _, cx| this.toggle_resolved_annotations(cx)),
                    )
                    .h_full()
                })),
        )
        .child(agent_strip(app, doc, cx))
        .child(body)
        .child(foot(app, doc, cx))
        .into_any_element()
}

/// What one row of the agent menu does.
#[derive(Clone, Copy)]
enum AgentRow {
    /// Open the New agent form, bound to this document on Start.
    New,
    Separator,
    Agent(AgentId),
    Unbind,
}

/// The agent menu's rows, in the order they are drawn and picked by: *New agent…* (a file's only —
/// a plan has its mission's coordinator), then the project's agents idle-first, then *Unbind*.
/// Read again by the pick handler, so an index names the row the user saw.
fn agent_rows(
    app: &AppState,
    doc: &DocumentEditor,
    cx: &App,
) -> Vec<(AgentRow, String, Option<gpui::Rgba>, bool)> {
    let mut rows = Vec::new();
    if doc.doc.rel_path().is_some() {
        rows.push((AgentRow::New, "New agent\u{2026}".to_string(), None, false));
    }
    let mut agents: Vec<_> = app
        .work(cx)
        .map(|work| {
            work.agents
                .iter()
                .map(|agent| {
                    let status = agent_status(agent, app.conversation(agent.id, cx));
                    (agent, status)
                })
                .collect()
        })
        .unwrap_or_default();
    // Idle (able to take a turn now) first, then the rest, each group by name.
    let rank = |lifecycle: Lifecycle| match lifecycle {
        Lifecycle::Idle | Lifecycle::Ready => 0,
        Lifecycle::Ended | Lifecycle::Unloaded => 2,
        _ => 1,
    };
    agents.sort_by_key(|(agent, status)| (rank(status.lifecycle), app.agent_label(agent).title));
    if !agents.is_empty() && !rows.is_empty() {
        rows.push((AgentRow::Separator, String::new(), None, false));
    }
    for (agent, status) in agents {
        rows.push((
            AgentRow::Agent(agent.id),
            app.agent_label(agent).title.to_string(),
            Some(card_colour(status)),
            rank(status.lifecycle) == 2,
        ));
    }
    if doc.binding.agent.is_some() {
        rows.push((AgentRow::Separator, String::new(), None, false));
        rows.push((AgentRow::Unbind, "Unbind".to_string(), None, false));
    }
    rows
}

/// The strip under the rail's header: the document's agent — its hexagon and name, or the button
/// that picks one — with where its queue stands, then the auto-send switch and *Ask agent*.
fn agent_strip(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let key = doc.surface_key();
    let view = cx.entity();
    let bound = doc
        .binding
        .agent
        .and_then(|id| app.work(cx).and_then(|work| work.agent(id)));
    let rows = agent_rows(app, doc, cx);
    let label = bound
        .map(|agent| app.agent_label(agent).title.to_string())
        .unwrap_or_else(|| match doc.binding.agent {
            // Bound to an agent this window's work no longer lists: the menu still opens, to rebind
            // or unbind.
            Some(_) => "Unknown agent".to_string(),
            None => "Agent".to_string(),
        });
    let kinds: Vec<AgentRow> = rows.iter().map(|row| row.0).collect();

    let mut picker = Picker::new(eid("doc-agent", &key), label)
        .style(PickerStyle::Plain)
        .anchor(gpui::Anchor::TopRight)
        .items(rows.iter().map(|row| row.1.clone()))
        .dots(rows.iter().map(|row| row.2))
        .dim(
            rows.iter()
                .enumerate()
                .filter(|(_, row)| row.3)
                .map(|(ix, _)| ix),
        )
        .separators(
            rows.iter()
                .enumerate()
                .filter(|(_, row)| matches!(row.0, AgentRow::Separator))
                .map(|(ix, _)| ix),
        )
        .open(app.workbench.open_menu == Some(MenuId::DocAgent))
        .tooltip("The agent working on this document")
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::DocAgent, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        // The rows as drawn, captured: an index names the row the user saw, not a fresh list.
        .on_pick(indexed(&view, move |this, index, window, cx| {
            let Some(kind) = kinds.get(index).copied() else {
                return;
            };
            match kind {
                AgentRow::New => {
                    this.close_menu(cx);
                    this.start_doc_agent(window, cx);
                }
                AgentRow::Agent(id) => this.set_doc_agent(Some(id), cx),
                AgentRow::Unbind => this.set_doc_agent(None, cx),
                AgentRow::Separator => {}
            }
        }));
    if doc.binding.agent.is_none() {
        picker = picker.icon(IconName::Bot);
    }
    if let Some(at) = rows
        .iter()
        .position(|row| matches!(row.0, AgentRow::Agent(id) if Some(id) == doc.binding.agent))
    {
        picker = picker.selected(at);
    }

    let mark = bound.map(|agent| {
        let status = agent_status(agent, app.conversation(agent.id, cx));
        status_mark(status, 14.0, eid("doc-agent-mark", &key))
    });
    // Where the agent's queue stands, said in words beside its name: nothing once delivered.
    let queue = doc
        .binding
        .agent
        .and_then(|id| app.workbench.doc_agent_queues.get(&id))
        .map(|(pending, state)| {
            let n = pending.len();
            match state {
                ubiq_proto::plan::DocDelivery::Waiting => format!("{n} waiting"),
                ubiq_proto::plan::DocDelivery::NotRunning => format!("{n} queued, not running"),
                ubiq_proto::plan::DocDelivery::Delivered => String::new(),
            }
        })
        .filter(|text| !text.is_empty());

    let auto = doc.binding.auto_send;
    let has_agent = doc.binding.agent.is_some();
    let toggle = has_agent.then(|| {
        icon_button(
            eid("doc-auto-send", &key),
            IconName::Bot,
            auto,
            cx.listener(|this, _, _, cx| this.toggle_doc_auto_send(cx)),
        )
        .tooltip(|window, cx| Tooltip::new("Send automatically to agent").build(window, cx))
    });
    let waiting = doc.awaiting_agent();
    let ask = (has_agent && !auto && waiting > 0).then(|| {
        ghost_button(
            eid("doc-ask-agent", &key),
            None,
            format!("Ask agent ({waiting})"),
            cx.listener(|this, _, _, cx| this.ask_doc_agent(cx)),
        )
        .tooltip(|window, cx| {
            Tooltip::new("Send the open threads waiting on the agent").build(window, cx)
        })
    });

    div()
        .flex_none()
        .h(px(HEADER))
        .px_3()
        .flex()
        .items_center()
        .gap_1p5()
        .border_b_1()
        .border_color(theme::border())
        .children(mark.map(|mark| div().flex_none().child(mark)))
        .child(picker)
        .children(queue.map(|text| {
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_size(theme::font(Family::Chrome, Role::Micro))
                .text_color(theme::text_faint())
                .child(text)
        }))
        .child(div().flex_1().min_w(px(0.)))
        .children(ask.map(|ask| div().flex_none().child(ask)))
        .children(toggle.map(|toggle| div().flex_none().child(toggle)))
        .into_any_element()
}

/// The rail's list, virtualized (T-152): a thread costs roughly as much to lay out as a block.
fn thread_list(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let key = doc.surface_key();
    let len = doc.rail_threads().len();
    if len == 0 {
        return div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .p_3()
            .child(note(
                "No threads here yet. Select a block and press + beside it.",
                theme::text_faint(),
            ))
            .into_any_element();
    }

    // `reset` is the only way to change a list's length and it drops the scroll position with the
    // measurements, so the reader's place is taken before and put back after.
    let list_state = app.plan_thread_list.clone();
    if list_state.item_count() != len {
        let at = list_state
            .logical_scroll_top()
            .item_ix
            .min(len.saturating_sub(1));
        list_state.reset(len);
        if at > 0 {
            list_state.scroll_to(gpui::ListOffset {
                item_ix: at,
                offset_in_item: px(0.),
            });
        }
    }
    let view = cx.entity();

    div()
        .id(eid("plan-annotations-list", &key))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .p_2()
        .child(
            list(list_state, move |index, window, cx| {
                thread_row(index, &view, window, cx)
            })
            .flex_1()
            .min_h(px(0.)),
        )
        .into_any_element()
}

/// The one row `gpui::list` asked for. Kept for the life of the list's `ListState`, so the document
/// is looked up fresh off `view` rather than borrowed from one frame's `AppState`.
fn thread_row(
    index: usize,
    view: &Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let app = view.read(cx);
    let Some(doc) = app.workbench.plan.as_ref() else {
        return div().into_any_element();
    };
    let Some(annotation) = doc
        .rail_threads()
        .get(index)
        .and_then(|id| doc.annotation(*id))
    else {
        return div().into_any_element();
    };
    let row = if doc.focused == Some(annotation.id) && annotation.is_open() {
        expanded(app, doc, annotation, view, window)
    } else {
        collapsed(doc, annotation, view, window)
    };
    // A row's own margin is invisible to `gpui::list`, so the gap between threads is padding.
    div().pb_2().child(row).into_any_element()
}

/// The edge a thread is drawn with: what it is waiting on, or that it lost its block.
fn edge(annotation: &Annotation) -> gpui::Rgba {
    if annotation.orphaned {
        theme::warning()
    } else {
        match annotation.state {
            AnnotationState::Open => theme::info(),
            AnnotationState::Resolved => theme::success(),
        }
    }
}

/// A thread that is not focused: its block's first line, its state, its marks and its first
/// comment on one line. A click focuses it and reveals its block; a resolved thread offers Reopen,
/// the only thing left to do with it.
fn collapsed(
    doc: &DocumentEditor,
    annotation: &Annotation,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let id = annotation.id;
    let resolved = annotation.state == AnnotationState::Resolved;
    let first = annotation
        .thread
        .first()
        .map(|comment| excerpt(&comment.text))
        .unwrap_or_default();

    let mut root = slab(edge(annotation))
        .id(eid("plan-thread", id))
        .p_2()
        .gap_1()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .on_click(window.listener_for(view, move |this, _, _, cx| this.focus_thread(id, cx)))
        .child(
            div()
                .flex()
                .items_start()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(state_line(doc, annotation)),
                )
                .children(resolved.then(|| {
                    ghost_button(
                        eid2("plan-reopen", id, "btn"),
                        None,
                        "Reopen",
                        window.listener_for(view, move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.set_annotation_resolved(id, false, cx);
                        }),
                    )
                })),
        );
    if !annotation.marks.is_empty() {
        root = root.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(annotation.marks.iter().copied().map(mark_badge)),
        );
    }
    root.child(
        div()
            .text_size(theme::font(Family::Content, Role::Meta))
            .text_color(theme::text_muted())
            .child(SharedString::from(first)),
    )
    .into_any_element()
}

/// The focused thread: its header (a click collapses it) with Resolve, its mark chips, every
/// comment, and the reply field — or the button that opens it.
fn expanded(
    app: &AppState,
    doc: &DocumentEditor,
    annotation: &Annotation,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let id = annotation.id;
    let replying = doc.composer == Some(ComposerTarget::Reply(id));

    let header = div()
        .id(eid("plan-thread-head", id))
        .flex()
        .items_start()
        .gap_1()
        .cursor_pointer()
        .on_click(window.listener_for(view, |this, _, _, cx| this.collapse_thread(cx)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(state_line(doc, annotation)),
        )
        // An agent proposed it done (`D208`): the user's two answers are Accept, which resolves,
        // and Reopen, which hands it back to the agent. Only the user ever resolves.
        .children(annotation.review.then(|| {
            ghost_button(
                eid2("plan-review-reopen", id, "btn"),
                None,
                "Reopen",
                window.listener_for(view, move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.reopen_for_agent(id, cx);
                }),
            )
        }))
        .child(ghost_button(
            eid2("plan-resolve", id, "btn"),
            Some(IconName::Check),
            if annotation.review {
                "Accept"
            } else {
                "Resolve"
            },
            window.listener_for(view, move |this, _, _, cx| {
                cx.stop_propagation();
                this.set_annotation_resolved(id, true, cx);
            }),
        ));

    let marks = div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(MARKS.into_iter().map(|mark| {
            toggle_pill(
                eid2("plan-thread-mark", id, mark_label(mark)),
                mark_label(mark),
                mark_colour(mark),
                annotation.marks.contains(&mark),
                window.listener_for(view, move |this, _, _, cx| {
                    this.toggle_annotation_mark(id, mark, cx)
                }),
            )
        }));

    let mut root = slab(theme::accent())
        .p_2()
        .gap_2()
        .child(header)
        .children(annotation.orphaned.then(orphan_banner))
        .child(marks)
        .children(
            annotation
                .thread
                .iter()
                .map(|comment| comment_row(app, doc, id, comment, view, window)),
        );

    root = if replying {
        root.child(composer_field(app, doc, "Reply", view))
    } else {
        root.child(div().flex().justify_end().child(ghost_button(
            eid2("plan-reply", id, "btn"),
            None,
            "Reply",
            window.listener_for(view, move |this, _, window, cx| {
                this.compose_reply(id, window, cx)
            }),
        )))
    };
    root.into_any_element()
}

/// What a thread is about and where it stands: its block's first line (or its quote), then how
/// many comments, and whether it is resolved or has lost its block.
fn state_line(doc: &DocumentEditor, annotation: &Annotation) -> AnyElement {
    let passage = annotation
        .quote
        .clone()
        .or_else(|| {
            doc.annotations
                .blocks()
                .iter()
                .find(|block| block.id == annotation.block_id)
                .map(|block| block.text.clone())
        })
        .map(|text| excerpt(&text))
        .unwrap_or_default();
    let count = annotation.thread.len();
    let mut state = format!("{count} comment{}", if count == 1 { "" } else { "s" });
    if annotation.state == AnnotationState::Resolved {
        state.push_str(" \u{b7} resolved");
    }
    if annotation.orphaned {
        state.push_str(" \u{b7} orphaned");
    }
    if annotation.review && annotation.state == AnnotationState::Open {
        state.push_str(" \u{b7} awaiting your review");
    }
    let colour = if annotation.orphaned {
        theme::warning()
    } else if annotation.review && annotation.state == AnnotationState::Open {
        theme::agent_controlled()
    } else if annotation.state == AnnotationState::Resolved {
        theme::success()
    } else {
        theme::text_muted()
    };
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(theme::font(Family::Content, Role::Meta))
                .text_color(theme::text())
                .child(SharedString::from(passage)),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(colour)
                .child(SharedString::from(state)),
        )
        .into_any_element()
}

/// The colour a mark is drawn in: the agent's own ink for `Agent`, a report colour otherwise.
fn mark_colour(mark: AnnotationMark) -> gpui::Rgba {
    match mark {
        AnnotationMark::Agent => theme::agent_controlled(),
        AnnotationMark::Todo => theme::warning(),
        AnnotationMark::Question => theme::info(),
    }
}

/// A mark as a small badge on a collapsed thread.
fn mark_badge(mark: AnnotationMark) -> AnyElement {
    let ground = match mark {
        AnnotationMark::Agent => theme::agent_controlled_soft(),
        AnnotationMark::Todo => theme::warning_soft(),
        AnnotationMark::Question => theme::info_soft(),
    };
    div()
        .flex_none()
        .px_1()
        .bg(ground)
        .text_size(theme::font(Family::Chrome, Role::Micro))
        .text_color(mark_colour(mark))
        .child(mark_label(mark))
        .into_any_element()
}

/// An annotation whose block is gone from the saved document — kept and shown, never dropped.
fn orphan_banner() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(Size::XSmall)
                .text_color(theme::warning()),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::warning())
                .child("This passage is no longer in the document."),
        )
        .into_any_element()
}

/// One comment, whole: who and when — an agent's in the agent's own ink, on its own ground — a
/// `→ agent` badge when it was addressed to one, then the text. The user's own comment offers
/// *Edit*, which swaps the text for the composer field seeded with it.
fn comment_row(
    app: &AppState,
    doc: &DocumentEditor,
    annotation_id: AnnotationId,
    comment: &Comment,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let editing = doc.composer == Some(ComposerTarget::Edit(annotation_id, comment.id));
    let comment_id = comment.id;
    let agent = comment.author == CommentAuthor::Agent;
    let (author, ink) = match comment.author {
        CommentAuthor::User => ("you", theme::text_muted()),
        CommentAuthor::Agent => ("agent", theme::agent_controlled()),
    };
    let when = comment.created_at.format("%Y-%m-%d %H:%M").to_string();
    let mut root = div().flex().flex_col().gap_0p5();
    if agent {
        root = root
            .px_1p5()
            .py_1()
            .bg(theme::agent_controlled_soft())
            .border_l(px(theme::accent_edge()))
            .border_color(theme::agent_controlled());
    }
    root.child(
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .font_family(theme::MONO_FONT)
                    .text_size(theme::font(Family::Conversation, Role::Label))
                    .text_color(ink)
                    .child(author),
            )
            .child(
                div()
                    .text_size(theme::font(Family::Conversation, Role::Meta))
                    .text_color(theme::text_faint())
                    .child(when),
            )
            .children(comment.to.map(|_| addressee_badge()))
            .children(comment.edited_at.map(|_| {
                div()
                    .text_size(theme::font(Family::Conversation, Role::Meta))
                    .text_color(theme::text_faint())
                    .child("edited")
            }))
            .child(div().flex_1().min_w(px(0.)))
            .children((!agent && !editing).then(|| {
                ghost_button(
                    eid2("plan-comment-edit", comment_id, "btn"),
                    None,
                    "Edit",
                    window.listener_for(view, move |this, _, window, cx| {
                        this.compose_edit(annotation_id, comment_id, window, cx)
                    }),
                )
            })),
    )
    .child(if editing {
        composer_field(app, doc, "Save", view)
    } else {
        div()
            .text_size(theme::font(Family::Conversation, Role::Body))
            .text_color(theme::text())
            .child(SharedString::from(comment.text.clone()))
            .into_any_element()
    })
    .into_any_element()
}

/// `→ agent`: a comment addressed to the agent.
fn addressee_badge() -> AnyElement {
    div()
        .px_1()
        .bg(theme::agent_controlled_soft())
        .text_size(theme::font(Family::Chrome, Role::Micro))
        .text_color(theme::agent_controlled())
        .child("\u{2192} agent")
        .into_any_element()
}

/// The rail's foot: the composer for a fresh thread while one is being written, a hint otherwise.
fn foot(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let key = doc.surface_key();
    let root = div()
        .flex_none()
        .flex()
        .flex_col()
        .gap_1p5()
        .p_2()
        .border_t_1()
        .border_color(theme::border());
    let Some(block_id) = doc.composer_block() else {
        return root
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child("Select a block, then + to start a thread on it."),
            )
            .into_any_element();
    };
    let passage = doc
        .annotations
        .blocks()
        .iter()
        .find(|block| block.id == block_id)
        .map(|block| excerpt(&block.text))
        .unwrap_or_default();
    let marks = div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(MARKS.into_iter().map(|mark| {
            toggle_pill(
                eid2("plan-composer-mark", &key, mark_label(mark)),
                mark_label(mark),
                mark_colour(mark),
                doc.composer_marks.contains(&mark),
                cx.listener(move |this, _, _, cx| this.toggle_composer_mark(mark, cx)),
            )
        }));
    root.child(
        slab(theme::accent())
            .p_2()
            .gap_1p5()
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child("New thread"),
            )
            .child(
                div()
                    .text_size(theme::font(Family::Content, Role::Meta))
                    .text_color(theme::text_muted())
                    .child(SharedString::from(passage)),
            )
            .child(marks)
            .child(composer_field(app, doc, "Send", &view)),
    )
    .into_any_element()
}

/// The one composer field, wherever it is shown — a fresh thread's at the rail's foot, or a reply
/// inside the focused thread; `DocumentEditor::composer` says which. The "@agent" chip addresses
/// the post to the agent, as a leading `@agent` in the text does.
fn composer_field(
    app: &AppState,
    doc: &DocumentEditor,
    send: &'static str,
    view: &Entity<AppState>,
) -> AnyElement {
    let key = doc.surface_key();
    let can_send = !doc.composer_text.trim().is_empty();
    let editing = matches!(doc.composer, Some(ComposerTarget::Edit(..)));
    let to_agent = doc.composer_to_agent && !editing;
    let act = |f: fn(&mut AppState, &mut Window, &mut Context<AppState>)| {
        let view = view.clone();
        move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
            view.update(cx, |this, cx| f(this, window, cx));
        }
    };
    let send_button = if can_send {
        primary_button(
            eid("plan-composer-send", &key),
            Some(IconName::Check),
            send,
            act(|this, window, cx| this.submit_annotation_composer(window, cx)),
        )
        .into_any_element()
    } else {
        ghost_button(eid("plan-composer-send", &key), None, send, |_, _, _| {})
            .text_color(theme::text_faint())
            .into_any_element()
    };
    let agent_chip = toggle_pill(
        eid("plan-composer-agent", &key),
        "@agent",
        theme::agent_controlled(),
        to_agent,
        act(|this, _, cx| this.toggle_composer_agent(cx)),
    )
    .tooltip(|window, cx| Tooltip::new("Address this comment to the agent").build(window, cx));

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .px_2()
                .py_1()
                .bg(theme::surface())
                .border_l(px(theme::accent_edge()))
                .border_color(if to_agent {
                    theme::agent_controlled()
                } else {
                    theme::border()
                })
                .child(Textarea::new(&app.annotation_composer_input)
                        .appearance(false)
                        .bordered(false)
                        .w_full()
                        .text_size(theme::font(Family::Conversation, Role::Body)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .children((!editing).then_some(agent_chip))
                .child(div().flex_1().min_w(px(0.)))
                .child(ghost_button(
                    eid("plan-composer-cancel", &key),
                    None,
                    "Cancel",
                    act(|this, window, cx| this.cancel_annotation_composer(window, cx)),
                ))
                .child(send_button),
        )
        .into_any_element()
}

/// The first line of `text`, cut to [`EXCERPT_CHARS`] with an ellipsis.
fn excerpt(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > EXCERPT_CHARS {
        let cut: String = line.chars().take(EXCERPT_CHARS).collect();
        format!("{cut}\u{2026}")
    } else {
        line.to_string()
    }
}

/// What the surface has to say about itself before anything else: the host's copy moved under an
/// unsaved edit, or Escape was pressed over one. Both are questions, and both keep the edit.
fn state_banner(doc: &DocumentEditor, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let key = doc.surface_key();
    if doc.stale {
        let who = match doc.stale_origin {
            Some(ubiq_proto::plan::SaveOrigin::Agent) => "An agent",
            Some(ubiq_proto::plan::SaveOrigin::Human) => "Someone else",
            None => "Something else",
        };
        let text = if doc.confirm_overwrite {
            format!(
                "{who} saved revision {} while you were editing. Save again to replace it with yours \u{2014} their edits are lost.",
                doc.host_revision,
            )
        } else {
            format!(
                "{who} saved revision {} while you were editing; you are at {}. Save asks again before replacing it.",
                doc.host_revision, doc.revision,
            )
        };
        return Some(warning_row(&key, text, None, cx));
    }
    if doc.confirm_close {
        return Some(warning_row(
            &key,
            "Unsaved changes. Save, or close again to discard.".to_string(),
            Some("Discard"),
            cx,
        ));
    }
    None
}

/// A sentence the surface has to put in front of the reader, in the warning colour — the stale
/// banner, the close question, and the viewer's own "editing the source can orphan a thread".
///
/// `discard` names the one button it may carry; `None` is a statement rather than a question.
/// `key` scopes the discard button's id to whichever surface raised it (T-125).
pub fn warning_row(
    key: &str,
    text: String,
    discard: Option<&'static str>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .bg(theme::warning_soft())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::warning())
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(Size::XSmall)
                .text_color(theme::warning()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text())
                .child(SharedString::from(text)),
        )
        .children(discard.map(|label| {
            ghost_button(
                eid("plan-discard", key),
                None,
                label,
                cx.listener(|this, _, _, cx| this.discard_plan_edits(cx)),
            )
        }))
        .into_any_element()
}

pub fn note(text: impl Into<SharedString>, colour: gpui::Rgba) -> AnyElement {
    div()
        .p_5()
        .text_size(theme::font(Family::Content, Role::Body))
        .text_color(colour)
        .child(text.into())
        .into_any_element()
}

/// The last one-shot action's outcome — an export that landed, a save the host refused — said
/// where the user is looking, in the colour of what it reports.
fn banner(notice: &Notice) -> AnyElement {
    let (text, bg, edge, icon) = match notice {
        Notice::Ok(said) => (
            said.clone(),
            theme::success_soft(),
            theme::success(),
            IconName::Check,
        ),
        Notice::Err(reason) => (
            reason.clone(),
            theme::danger_soft(),
            theme::danger(),
            IconName::TriangleAlert,
        ),
    };
    div()
        .px_2()
        .py_1()
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .bg(bg)
        .border_l(px(theme::accent_edge()))
        .border_color(edge)
        .child(Icon::new(icon).with_size(Size::XSmall).text_color(edge))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text())
                .child(SharedString::from(text)),
        )
        .into_any_element()
}
