//! The centre pane: the open files' tabs, and the editor under them.

use gpui::{Context, IntoElement, ParentElement, Styled, div, px, relative};
use gpui_component::highlighter::Language;
use gpui_component::input::Editor;

use crate::app::AppState;
use crate::state::FileLanguage;
use crate::theme;
use crate::ui::explorer::git_colour;
use crate::ui::indexed;
use crate::ui::kit::{Tab, tab_strip};

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

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let view = cx.entity();
    let tabs: Vec<Tab> = app
        .editor
        .open
        .iter()
        .map(|file| {
            Tab::new(file.name.clone())
                .dot(git_colour(file.git))
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
            app.editor.active,
            indexed(&view, |this, index, window, cx| {
                this.activate_editor_tab(index, window, cx)
            }),
            Some(std::rc::Rc::new(indexed(
                &view,
                |this, index, window, cx| this.close_editor_tab(index, window, cx),
            ))),
            None,
        ))
        .child(
            div().flex().flex_1().min_h(px(0.)).child(
                Editor::new(&app.editor_state)
                    .h(relative(1.))
                    .p_0()
                    .border_0(),
            ),
        )
}
