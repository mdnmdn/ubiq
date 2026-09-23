//! The plan editor: the annotated-document surface, framed as a dialog.
//!
//! **The surface itself is `crate::ui::document`'s** — the preview, the per-section gutter, the
//! thread rail, the composer and the section editor, shared with the markdown viewer's fourth
//! layout (`ViewLayout::Annotation`, T-124). What is left here is what is true of *this* frame and
//! of nothing else: the modal, its title, the chrome strip above the document and the footer
//! beneath it, where a plan is saved and exported from.
//!
//! **The plan editor stays a dialog** — a user ruling, not an accident of history: a plan is
//! raised from a task and answered, rather than kept open beside the code the way a file's
//! annotations are.
//!
//! **Nothing here names a plan.** The surface reads a `DocumentEditor`, whose handle says which
//! document is open; the title is the one place a plan's task is looked up, and it asks the
//! handle rather than assuming.

use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::Layer;
use crate::state::document::DocumentEditor;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::document::{MINIMAP_WIDTH, RAIL_WIDTH};
use crate::ui::kit::{
    UbiqIcon, ghost_button, icon_button, modal_sized, primary_button, status_dot,
};
use ubiq_proto::plan::SaveOrigin;

/// Wide enough to write in beside the thread rail — a writing surface, not a dialog. Forty
/// narrower than before the minimap: the strip's own cost is split between the two, not loaded
/// onto the document alone.
const DOC_WIDTH: f32 = 820.0;
/// Both columns need a real height to scroll inside — `modal_sized`'s `fill_height`.
const DOC_HEIGHT: f32 = 700.0;
/// The chrome strip above the document, the file viewer's own header height.
const HEADER: f32 = 32.0;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(doc) = app.workbench.plan.as_ref().filter(|doc| doc.is_modal()) else {
        return div().into_any_element();
    };

    let title = match doc.task_id().and_then(|id| {
        app.work(cx)
            .and_then(|work| work.task(id))
            .map(|task| task.title.clone())
    }) {
        Some(task) => format!("{} \u{2014} {task}", doc.doc.kind_label()),
        None => doc.doc.kind_label().to_string(),
    };

    let view = cx.entity();
    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(chrome(app, doc, &view))
        .child(crate::ui::document::surface(app, doc, cx))
        .into_any_element();

    modal_sized(
        "plan-modal",
        theme::accent(),
        DOC_WIDTH + RAIL_WIDTH + MINIMAP_WIDTH,
        Some(DOC_HEIGHT),
        &title,
        body,
        footer(doc, cx),
        crate::ui::dismiss(&view, Layer::Plan, |this, _, cx| this.close_plan(cx)),
        window,
    )
}

/// The strip above the document: what the document is holding, and the one thing to press — the
/// heading navigator, which is also how many threads are open at a glance.
///
/// **There is no `+ annotation` here.** Annotating is a section's own affordance, beside the
/// section it is about, and there is only one way to draw the document. The minimap's show/hide
/// control is the one layout toggle this strip carries — a reading habit, not a document fact.
fn chrome(app: &AppState, doc: &DocumentEditor, view: &Entity<AppState>) -> AnyElement {
    let annotating = doc.composer_block().is_some();

    div()
        .h(px(HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .bg(theme::pane_bg())
        // `above_modal`: this trigger sits inside a modal, so its panel paints over the modal's
        // own overlay rather than under it.
        .child(crate::ui::document::navigator(doc, true, view))
        .child(div().flex_1().min_w(px(0.)))
        .children(annotating.then(|| {
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::accent())
                .child("writing a thread")
        }))
        .child({
            let view = view.clone();
            icon_button(
                "plan-minimap-toggle",
                UbiqIcon::TitlebarPanelLeft,
                app.workbench.settings.ui.md_minimap,
                move |_event, _window, cx| {
                    view.update(cx, |this, cx| this.toggle_md_minimap(cx));
                },
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Show or hide the thread minimap")
                    .build(window, cx)
            })
        })
        .into_any_element()
}

/// The foot of the surface: what the buffer is, then what can be done to it. Save is the primary
/// action here for the same reason ⌘S reaches it — this is an editor.
fn footer(doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let (label, colour) = if doc.saving {
        ("Saving\u{2026}", theme::text_faint())
    } else if doc.dirty {
        ("Unsaved changes", theme::warning())
    } else {
        ("Saved", theme::text_faint())
    };

    div()
        .flex()
        .flex_1()
        .items_center()
        .gap_2()
        .child(status_dot(colour, theme::transparent()))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(label),
        )
        .children(change_summary(doc))
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "plan-export",
            Some(IconName::FileText),
            "Export\u{2026}",
            cx.listener(|this, _, window, cx| this.open_export_plan_dialog(window, cx)),
        ))
        .child(save_button(doc, cx))
        .into_any_element()
}

/// What the edit-provenance layer has to say, in one line of the footer: how much changed, and
/// which of the two underline hues in the buffer are on screen.
///
/// Counts, not a report. `PlanChangeStats` carries more than this — the revisions in the window,
/// split by who made them — and a footer that showed all of it would be a dashboard in a status
/// bar. Absent entirely for a document nothing has happened to, which is the ordinary case.
fn change_summary(doc: &DocumentEditor) -> Option<AnyElement> {
    let stats = &doc.change_stats;
    if stats.is_empty() && stats.blocks_touched == 0 {
        return None;
    }
    let counts = format!(
        "+{} \u{2212}{} \u{223c}{} lines \u{00b7} {} block{}",
        stats.lines_added,
        stats.lines_removed,
        stats.lines_modified,
        stats.blocks_touched,
        if stats.blocks_touched == 1 { "" } else { "s" },
    );
    let mut row = div().flex().items_center().gap_1p5().child(
        div()
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .text_color(theme::text_faint())
            .child(SharedString::from(counts)),
    );
    for (origin, label) in [
        (SaveOrigin::Human, "you"),
        (SaveOrigin::Agent, SaveOrigin::Agent.label()),
    ] {
        if !doc.has_changes_by(origin) {
            continue;
        }
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(status_dot(
                    theme::edit_origin(origin.is_human()),
                    theme::transparent(),
                ))
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                        .text_color(theme::text_faint())
                        .child(label),
                ),
        );
    }
    Some(row.into_any_element())
}

fn save_button(doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    if doc.dirty || doc.stale {
        // The button says what the next press does. A stale buffer's first press only asks.
        let label = if doc.confirm_overwrite {
            "Overwrite"
        } else {
            "Save"
        };
        primary_button(
            "plan-save",
            Some(IconName::Check),
            label,
            cx.listener(|this, _, window, cx| this.save_document(window, cx)),
        )
        .into_any_element()
    } else {
        ghost_button("plan-save", Some(IconName::Check), "Save", |_, _, _| {}).into_any_element()
    }
}
