//! The centre pane: the open files' tabs, and what each one is showing.
//!
//! A tab is not always a buffer. It exists from the click that asked for the file, so a click has
//! an effect straight away, and what it draws until the bytes arrive — or instead of them, when the
//! read failed or the file is not text — is this module's business.

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Rgba, SharedString, Styled, div, px, relative,
};
use gpui_component::highlighter::Language;
use gpui_component::input::Editor;

use crate::app::AppState;
use crate::state::{FileBody, FileLanguage, OpenFile, SaveState};
use crate::theme;
use crate::ui::indexed;
use crate::ui::kit::{Tab, mono, tab_strip};

/// The highlighter's language for one of ours. This is the only place the two enums meet.
pub fn highlighter_language(language: FileLanguage) -> Language {
    match language {
        FileLanguage::Tsx => Language::Tsx,
        FileLanguage::TypeScript => Language::TypeScript,
        FileLanguage::Json => Language::Json,
        FileLanguage::Rust => Language::Rust,
        FileLanguage::Markdown => Language::Markdown,
        FileLanguage::Plain => Language::Plain,
    }
}

/// What a tab's dot reports: the file, not the repository.
///
/// Nothing reads version control, so this is the file's own state — whether it is still arriving,
/// whether it is on its way to disk, whether that failed, and whether it holds an unsaved edit.
fn state_colour(file: &OpenFile) -> Rgba {
    match (&file.save, &file.body) {
        (SaveState::Failed(_), _) => theme::danger(),
        (SaveState::Saving(_), _) => theme::info(),
        (_, FileBody::Failed(_)) => theme::danger(),
        (_, FileBody::Loading) => theme::text_faint(),
        _ if file.dirty() => theme::warning(),
        _ => theme::text_muted(),
    }
}

/// The tab's label. A dirty file is marked in shape as well as colour, because a dot alone is not
/// something to rely on.
fn label(file: &OpenFile, confirming: bool) -> SharedString {
    if confirming {
        return SharedString::from(format!("{} \u{2014} discard?", file.name));
    }
    match file.dirty() {
        true => SharedString::from(format!("{} \u{2022}", file.name)),
        false => SharedString::from(file.name.clone()),
    }
}

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    // The tabs are the project's, not the window's: each project this window holds keeps its own
    // open files, and switching between them is a lookup.
    let Some(editor) = app.editor(cx) else {
        return div().into_any_element();
    };
    let confirming = editor.pending_tab_close.clone();

    let tabs: Vec<Tab> = editor
        .open
        .iter()
        .map(|file| {
            let asking = confirming.as_deref() == Some(file.path.as_str());
            Tab::new(label(file, asking))
                .dot(state_colour(file))
                .closable(true)
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(tab_strip(
            "editor-tab",
            tabs,
            editor.active,
            indexed(&view, |this, index, _, cx| {
                this.activate_editor_tab(index, cx)
            }),
            Some(std::rc::Rc::new(indexed(&view, |this, index, _, cx| {
                this.close_editor_tab(index, cx)
            }))),
            None,
        ))
        .child(div().flex().flex_1().min_h(px(0.)).child(body(app, cx)))
        .into_any_element()
}

/// The active file, in whatever state it is in.
fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(file) = app.editor(cx).and_then(|editor| editor.active_file()) else {
        return note("No file open", theme::text_faint());
    };

    match &file.body {
        FileBody::Text { state, .. } => Editor::new(state)
            .h(relative(1.))
            .p_0()
            .border_0()
            .into_any_element(),
        // The header is already drawn above; the body stays empty until the bytes land.
        FileBody::Loading => note("Reading\u{2026}", theme::text_faint()),
        FileBody::Binary => note("Not text \u{b7} nothing to show", theme::text_faint()),
        FileBody::Failed(reason) => note(reason.clone(), theme::danger()),
    }
}

fn note(text: impl Into<SharedString>, colour: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .child(mono(text, colour))
        .into_any_element()
}
