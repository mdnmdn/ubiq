//! The knowledge base on screen: the explorer that lists every source, and the panel that draws
//! one open document.
//!
//! This is the documents half of the IDE, and it borrows the IDE's parts rather than restating
//! them — [`crate::ui::kit::files`]'s row, twisty and kind icon draw a line here exactly as they
//! draw one in `ui/explorer.rs`.
//!
//! **A document is a dock tab, exactly as a file is.** It is the same `OpenFile`, held in
//! `KbState::docs`, drawn by [`crate::ui::viewer`], and given its tab strip, its dirty dot, its
//! drag, its pin and its close by the dock — so several documents are open at once and none of
//! the IDE's chrome is said twice here. What this module keeps is the explorer, the body lookup
//! ([`render_doc`]) and the page that says nothing is open yet.
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
use crate::state::kb::{KbRow, KbRowKind};
use crate::theme;
use crate::ui::kit::{
    ContextItem, UbiqIcon, context_menu, elided, elided_with, file_row, icon_button, kind_icon,
    panel, panel_header, primary_button, row_font, twisty,
};
use crate::ui::mark;
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

/// The centre with no document open: the explorer is what opens one, so that is what it points
/// at. As soon as a document is open the document panels *are* the centre and this steps aside,
/// exactly as `ui/editor.rs`'s welcome page does in IDE mode.
pub fn centre(app: &AppState, _window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let open = app.kb(cx).is_some_and(|kb| !kb.docs.is_empty());
    if open {
        // Reached for the frame between a tab opening and the dock settling its panel.
        return div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .bg(theme::app_bg())
            .into_any_element();
    }
    mark::backdrop(
        app,
        empty::empty_page(
            "No document open",
            "Pick one from the documents explorer on the left.",
            UbiqIcon::ModeKb,
            None,
        )
        .into_any_element(),
        cx,
    )
}

/// One document panel's body: the document its tab key names, drawn by its viewer.
///
/// **The same viewer seam the IDE uses.** A document is an `OpenFile`, so what draws it is
/// `ViewerKind`'s answer and everything past the lookup is `ui/viewer/`'s — the highlighted
/// buffer, the Markdown render, the layout toggle. The header naming the document is gone with
/// the single-document centre it existed for: the dock's tab strip says which document this is,
/// and says it for every one of them at once.
pub fn render_doc(app: &AppState, key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let Some(doc) = app.kb_doc(key, cx) else {
        // A panel whose tab has gone is hidden rather than drawn, so this is the frame between
        // the two.
        return crate::ui::viewer::note("No document open", theme::text_faint());
    };
    // Every `ViewerKind` now draws here: an image is `FileBody::Bytes` handed straight to
    // `ui/viewer/image.rs`, exactly as a project file's is, and a diagram is the same buffer and
    // the same `Edit` layout that opens the web-panel bridge for one (`T-32`). `ui/viewer/`
    // decides what to draw; nothing here decides for it.
    crate::ui::viewer::render(app, doc, cx)
}
