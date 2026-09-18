//! The knowledge base on screen: the explorer that lists every source, and the centre that draws
//! the document it selects.
//!
//! This is the documents half of the IDE, and it borrows the IDE's parts rather than restating
//! them — [`crate::ui::kit::files`]'s row, twisty and kind icon draw a line here exactly as they
//! draw one in `ui/explorer.rs`, and the centre hands Markdown to
//! [`crate::ui::viewer::markdown`], which is the one Markdown renderer in the window.
//!
//! **A source is not a folder in a tree, it is a place the documents come from**, so its row says
//! where — the origin as a tooltip — and how far it has got. A source that is not readable yet is
//! a colour and a word from [`ubiq_proto::kb::KbSourceState`], and its row retries rather than
//! doing nothing.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_component::IconName;

use ubiq_proto::ids::KbSourceId;
use ubiq_proto::kb::KbSourceState;

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::editor::ViewerKind;
use crate::state::kb::{KbBody, KbDoc, KbRow, KbRowKind};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    ContextItem, UbiqIcon, context_menu, elided, elided_with, file_row, icon_button, kind_icon,
    mono, panel, panel_header, primary_button, row_font, row_height, twisty,
};
use crate::ui::viewer::markdown;
use crate::ui::{eid2, empty};

pub mod source_form;

/// The left panel: the sources, and the folders and files under the open ones.
pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let header = panel_header(
        "Knowledge base",
        icon_button(
            "kb-settings",
            IconName::Settings,
            false,
            cx.listener(|this, _, _, cx| this.open_kb_settings(cx)),
        ),
    );

    let body = panel().border_r_1().border_color(theme::border());
    let Some(kb) = app.kb(cx) else {
        return body.child(header).into_any_element();
    };

    // Not configured yet is not the same as configured-and-empty: the first is a panel waiting for
    // an answer, and drawing the entry point over it would offer to add what may already be there.
    if !kb.loaded {
        return body.child(header).into_any_element();
    }

    if kb.sources.is_empty() {
        return body.child(header).child(add_page(cx)).into_any_element();
    }

    let font_size = row_font();
    let rows: Vec<AnyElement> = kb
        .rows()
        .iter()
        .map(|row| line(row, font_size, cx))
        .collect();

    let mut body = body.child(header).child(
        div()
            .id("kb-tree")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .children(rows),
    );

    // The menu the project explorer draws, drawn here: same kit panel, same index-into-`entries`
    // pick, same epoch-carrying dismissal.
    if app.workbench.open_menu == Some(MenuId::Kb)
        && let Some(menu) = kb.menu.clone()
    {
        let epoch = menu.epoch;
        let items: Vec<ContextItem> = menu
            .entries()
            .into_iter()
            .map(|action| match action.is_separator() {
                true => ContextItem::separator(),
                false => ContextItem::new(action.label()),
            })
            .collect();
        body = body.child(context_menu(
            "kb-menu",
            point(px(menu.x), px(menu.y)),
            items,
            crate::ui::indexed(&cx.entity(), |this, index, window, cx| {
                this.pick_kb_action(index, window, cx);
            }),
            crate::ui::handler(&cx.entity(), move |this, _, cx| {
                this.dismiss_kb_menu(epoch, cx)
            }),
        ));
    }

    body.into_any_element()
}

/// The whole entry point, and the first thing a new project's knowledge base says. Filled rather
/// than ghosted: there is one action on this screen and nothing else to do until it is taken.
fn add_page(cx: &mut Context<AppState>) -> AnyElement {
    empty::empty_page(
        "No knowledge base",
        "Point Ubiq at the folders and repositories this project's documents live in, and they \
         are read here and by its agents.",
        UbiqIcon::ModeKb,
        Some(
            primary_button(
                "kb-add",
                Some(IconName::Plus),
                "Add KB",
                cx.listener(|this, _, _, cx| this.open_kb_settings(cx)),
            )
            .into_any_element(),
        ),
    )
    .into_any_element()
}

/// One row of the flattened tree. A source leads with its state; a folder and a file read as they
/// do in the project explorer.
fn line(row: &KbRow, font_size: f32, cx: &mut Context<AppState>) -> AnyElement {
    let source = row.source;
    let path = row.path.clone();
    let mut line = file_row(
        eid2("kb-row", source, &row.path),
        row.depth,
        row.selected,
        false,
        false,
        font_size,
    )
    .on_click(cx.listener(move |this, _, _, cx| {
        this.click_kb_row(source, path.clone(), cx);
    }));

    let menu_path = row.path.clone();
    line = line.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.open_kb_menu(
                source,
                menu_path.clone(),
                (f32::from(event.position.x), f32::from(event.position.y)),
                cx,
            );
        }),
    );

    match &row.kind {
        KbRowKind::Root {
            expanded,
            state,
            origin,
            error,
        } => {
            let toggled = row.source;
            line = line
                .child(twisty(
                    eid2("kb-twisty", source, "source"),
                    *expanded,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.click_kb_row(toggled, String::new(), cx);
                    }),
                ))
                .child(kind_icon(true, theme::text_muted()))
                // The name is elided; the origin is what the hover answers, because "which docs
                // folder is this" is a question the name alone often cannot settle.
                .child(elided_with(
                    eid2("kb-name", source, "source"),
                    row.name.clone(),
                    origin.clone(),
                    theme::text(),
                    px(font_size),
                ))
                .children(state_word(source, state, font_size))
                // What the last gesture was refused for. On the source's row because a create,
                // a rename or a delete that failed has no document on screen to fail in.
                .children(error.as_ref().map(|error| {
                    elided_with(
                        eid2("kb-error", source, "error"),
                        error.clone(),
                        error.clone(),
                        theme::danger(),
                        px(font_size),
                    )
                    .flex_none()
                    .into_any_element()
                }))
                .children(retry(source, state, cx));
        }
        KbRowKind::Dir { expanded, loading } => {
            let toggled = row.path.clone();
            line = line
                .child(twisty(
                    eid2("kb-twisty", source, &row.path),
                    *expanded,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.click_kb_row(source, toggled.clone(), cx);
                    }),
                ))
                .child(kind_icon(true, theme::text_muted()))
                .child(elided_with(
                    eid2("kb-name", source, &row.path),
                    row.name.clone(),
                    row.path.clone(),
                    theme::text(),
                    px(font_size),
                ))
                .children(loading.then(|| {
                    elided(
                        eid2("kb-loading", source, &row.path),
                        "\u{2026}",
                        theme::text_faint(),
                        px(font_size),
                    )
                }));
        }
        KbRowKind::File => {
            line = line
                .child(kind_icon(false, theme::text_muted()))
                .child(elided_with(
                    eid2("kb-name", source, &row.path),
                    row.name.clone(),
                    row.path.clone(),
                    theme::text(),
                    px(font_size),
                ));
        }
    }

    line.into_any_element()
}

/// What a source says beside its name, in the colour its state earns. `Ready` says nothing: a
/// source that works is the ordinary case, and a word for it would be noise on every row.
fn state_word(source: KbSourceId, state: &KbSourceState, font_size: f32) -> Option<AnyElement> {
    let (word, colour, tooltip) = match state {
        KbSourceState::Ready => return None,
        KbSourceState::Pending => ("pending".to_string(), theme::text_faint(), None),
        KbSourceState::Syncing { detail } => (detail.clone(), theme::warning(), None),
        KbSourceState::Failed { error } => ("failed".to_string(), theme::danger(), Some(error)),
    };
    let tooltip: SharedString = match tooltip {
        Some(error) => error.clone().into(),
        None => word.clone().into(),
    };
    Some(
        elided_with(
            eid2("kb-state", source, "state"),
            word,
            tooltip,
            colour,
            px(font_size),
        )
        .flex_none()
        .into_any_element(),
    )
}

/// The one control a source that cannot be read offers. A press on the row does the same thing,
/// and this is what makes that discoverable.
fn retry(
    source: KbSourceId,
    state: &KbSourceState,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    if matches!(state, KbSourceState::Ready | KbSourceState::Syncing { .. }) {
        return None;
    }
    Some(
        icon_button(
            eid2("kb-retry", source, "retry"),
            IconName::RotateCw,
            false,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.sync_kb_source(source, cx);
            }),
        )
        .into_any_element(),
    )
}

/// The centre: the selected document, drawn by whichever renderer its extension names.
///
/// **Leads with a header naming the document**, unlike the IDE editor, which leans on the dock's
/// own tab strip for that — the knowledge base has no tab strip here, so a click with nothing
/// else to show for it (a document just created, empty, on a wiki) would otherwise look like it
/// did nothing at all. The header is what says a click was heard even when the body has nothing
/// to draw.
pub fn centre(app: &AppState, _window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(kb) = app.kb(cx) else {
        return nothing_selected();
    };
    let Some(doc) = &kb.doc else {
        return nothing_selected();
    };

    let body = match &doc.body {
        KbBody::Loading => faint("Opening\u{2026}"),
        KbBody::Failed(error) => sentence(error.clone(), theme::danger()),
        KbBody::Ready(contents) => {
            if contents.is_binary {
                faint(format!("{} is not text.", doc.key.name()))
            } else {
                let source = String::from_utf8_lossy(&contents.bytes).into_owned();
                // The knowledge base's own key space, so a document and an editor tab on a file of
                // the same name are two entries in the renderer's scan cache rather than one.
                let key = format!("kb:{}:{}", doc.key.source, doc.key.path);
                let font_size = app.content_font_size(cx);
                match ViewerKind::of(&doc.key.path) {
                    ViewerKind::Markdown => {
                        markdown::render(app, &key, &source, font_size, false, cx)
                    }
                    ViewerKind::Editor => plain(source, font_size),
                    // Diagrams and images are the IDE's viewers, and reaching them from here means
                    // wiring a web tenant to a document that is not an open file. Until that is
                    // done the panel says where the file is drawn rather than drawing it wrongly.
                    _ => faint(format!("{} opens in the IDE.", doc.key.name())),
                }
            }
        }
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(doc_header(doc, kb))
        .child(body)
        .into_any_element()
}

/// The flush row naming the open document, the one thing on screen that changes the instant a
/// click lands — a `panel_header`'s own reasoning, spelled here rather than reused because a
/// document's title is a path, not an uppercase section name.
fn doc_header(doc: &KbDoc, kb: &crate::state::kb::KbState) -> AnyElement {
    let title = match kb.source(doc.key.source) {
        Some(view) => format!("{} / {}", view.name(), doc.key.path),
        None => doc.key.path.clone(),
    };
    let font_size = row_font();
    div()
        .h(px(row_height(font_size)))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(theme::border())
        .bg(theme::pane_bg())
        .child(kind_icon(false, theme::text_muted()))
        .child(elided_with(
            eid2("kb-doc-title", doc.key.source, &doc.key.path),
            doc.key.name().to_string(),
            title,
            theme::text(),
            px(font_size),
        ))
        .into_any_element()
}

/// The centre with no document chosen: the explorer is what chooses one, so that is what it points
/// at.
fn nothing_selected() -> AnyElement {
    empty::empty_page(
        "No document open",
        "Pick one from the documents explorer on the left.",
        UbiqIcon::ModeKb,
        None,
    )
    .into_any_element()
}

/// A document with no renderer of its own, as its own text.
fn plain(source: String, font_size: Option<f32>) -> AnyElement {
    div()
        .id("kb-plain")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p_5()
        .text_size(px(font_size.unwrap_or(theme::EDITOR_FONT_SIZE)))
        .text_color(theme::text())
        .child(mono(source, theme::text()))
        .into_any_element()
}

fn faint(note: impl Into<String>) -> AnyElement {
    empty::empty_panel(&note.into()).into_any_element()
}

fn sentence(text: String, colour: gpui::Rgba) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .p_5()
        .justify_center()
        .items_center()
        .child(
            div()
                .max_w(px(420.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(colour)
                .child(SharedString::from(text)),
        )
        .into_any_element()
}
