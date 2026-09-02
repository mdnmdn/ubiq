//! One open file, and what it is showing.
//!
//! A tab is not always a buffer. It exists from the click that asked for the file, so a click has
//! an effect straight away, and what it draws until the bytes arrive — or instead of them, when the
//! read failed or the file is not text — is this module's business.
//!
//! **The tabs are the dock's.** Each open file is its own panel, so its tab belongs to the group it
//! sits in and a file can be dragged beside another. What is left here is what one file draws, and
//! the two things its tab asks of the file it names: [`label`] and [`state_colour`]. The centre
//! panel keeps only the page that says no file is open, which is what it is in IDE mode.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Image, ImageFormat, ImageSource, IntoElement, ParentElement, Rgba,
    SharedString, Styled, div, img, px,
};
use gpui_component::highlighter::Language;

use crate::app::AppState;
use crate::state::{FileBody, FileLanguage, OpenFile, SaveState};
use crate::theme::{self, ThemeId};
use crate::ui::kit::mono;

/// The page's logo files, theme picked so it reads on the page's background: the blue mark for a
/// light theme, the white for a dark one. They are Ubiq's only assets, baked in wherever drawn.
const LOGO_WHITE: &[u8] = include_bytes!("../../../../assets/logo-white.png");
const LOGO_BLUE: &[u8] = include_bytes!("../../../../assets/logo-blue.png");

/// The highlighter's language for one of ours. This is the only place the two enums meet.
pub fn highlighter_language(language: FileLanguage) -> Language {
    match language {
        FileLanguage::Tsx => Language::Tsx,
        FileLanguage::TypeScript => Language::TypeScript,
        FileLanguage::Json => Language::Json,
        FileLanguage::Rust => Language::Rust,
        FileLanguage::Markdown => Language::Markdown,
        FileLanguage::Yaml => Language::Yaml,
        FileLanguage::Java => Language::Java,
        FileLanguage::Plain => Language::Plain,
    }
}

/// What a tab's dot reports: the file, not the repository.
///
/// Nothing reads version control, so this is the file's own state — whether it is still arriving,
/// whether it is on its way to disk, whether that failed, and whether it holds an unsaved edit.
pub fn dirty_colour(file: &OpenFile) -> Rgba {
    match (&file.save, &file.body) {
        (SaveState::Failed(_), _) => theme::danger(),
        (SaveState::Saving(_), _) => theme::info(),
        (_, FileBody::Failed(_)) => theme::danger(),
        (_, FileBody::Loading) => theme::text_faint(),
        _ if file.dirty() => theme::warning(),
        _ => theme::text_muted(),
    }
}

/// The colour a file tab's title takes from the repository, the same mapping the explorer uses.
pub fn git_colour(file: &OpenFile, explorer: &crate::state::ExplorerState) -> Rgba {
    crate::ui::explorer::git_colour(explorer.git_status(&file.path))
}

/// The tab's label. The dot and git colour are drawn by the dock's tab skin, so the label itself
/// is just the name — the file's own report lives next to it, not inside it.
pub fn label(file: &OpenFile, confirming: bool) -> SharedString {
    let name = format!("{}{}", file.name, file.subject.suffix());
    if confirming {
        return SharedString::from(format!("{name} \u{2014} discard?"));
    }
    SharedString::from(name)
}

/// The centre panel in IDE mode, which is only ever the page saying no file is open.
///
/// As soon as one is, the file panels are the centre and this one steps aside — hidden rather than
/// removed, so it comes back where it was left when the last tab closes.
pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let open = app.editor(cx).is_some_and(|editor| !editor.open.is_empty());
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(match open {
            // Reached for the frame between a tab opening and the dock settling its panel.
            true => note("\u{2026}", theme::text_faint()),
            false => welcome(app),
        })
        .into_any_element()
}

/// The page when no file is open in IDE mode: the brand mark, big and soft, on the empty page.
///
/// A surface that is about to hold a file says so without furniture — no button, no menu, just the
/// mark that owns the window. The logo is theme picked (blue on light, white on dark) so it reads
/// on the page's background exactly as the rail's does on its swatch.
fn welcome(app: &AppState) -> AnyElement {
    let logo = Arc::new(Image::from_bytes(
        ImageFormat::Png,
        match app.workbench.theme_id {
            ThemeId::Light => LOGO_BLUE,
            ThemeId::Dark => LOGO_WHITE,
        }
        .to_vec(),
    ));
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .bg(theme::app_bg())
        .child(
            div()
                .size(px(200.))
                .opacity(0.5)
                .child(img(ImageSource::Image(logo)).size_full()),
        )
        .into_any_element()
}

/// One file panel's body: the file its tab key names, drawn by its viewer.
///
/// **The viewer seam.** What draws a file is `ViewerKind`'s answer rather than this function's
/// shape, so everything past the lookup is `ui/viewer/`'s — including the plain text case, which
/// is the editor viewer among the others rather than the arm the rest fall out of.
pub fn render_file(app: &AppState, key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let Some(file) = app.file(key, cx) else {
        // A panel whose tab has gone is hidden rather than drawn, so this is the frame between
        // the two.
        return note("No file open", theme::text_faint());
    };
    super::viewer::render(app, file, cx)
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
