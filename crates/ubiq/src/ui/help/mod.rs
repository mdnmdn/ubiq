//! The help panel: Ubiq's own documentation, drawn.
//!
//! A header, a contents tree, and one page. The page is drawn by the window's one markdown
//! renderer — `viewer::markdown::render_linked` — so a help page gets tables, task lists, code
//! fences and Mermaid diagrams for free, and there is no second renderer to keep in step.
//!
//! **A link is a page id before it is a path.** That is the documented form, and it is what makes
//! moving a page between folders harmless. A target the catalogue does not answer is tried as a
//! path relative to the page's own folder, and only then handed to [`crate::ui::on_link`], which
//! is what sends `ubiq://` somewhere and the web to the browser.
//!
//! **Nothing here can fail.** No content built draws the stub compiled in beside this file; a page
//! the nav lists with no file behind it says so and keeps its place in the tree; a planned page is
//! greyed and opens on the same body. There is no error dialog anywhere in the panel.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::help::{HelpCatalog, HelpStatus};

use crate::app::AppState;
use crate::state::help::HelpState;
use crate::theme;
use crate::ui::kit::{self, UbiqIcon, elided, icon_button, panel, row_height, row_indent, twisty};
use crate::ui::{self, eid, viewer};

/// What the panel shows when this build has no documentation in it. Compiled in rather than
/// fetched, because the whole point of it is to be there when nothing else is.
const STUB: &str = include_str!("unavailable.md");

/// How wide the contents tree is when it is open. A constant here rather than a resizable split:
/// the tree is a list of page titles, and the page beside it is what the reader came for.
const CONTENTS_WIDTH: f32 = 200.;

/// The panel.
pub fn render(app: &AppState, _window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    panel()
        .child(header(app, cx))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h(px(0.))
                .when(app.help.contents_open, |this| this.child(contents(app, cx)))
                .child(body(app, cx)),
        )
        .into_any_element()
}

/// The row above the page: where the reader is, how they got here, and the contents toggle.
fn header(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let title = app
        .help
        .current()
        .map(|page| page.title.clone())
        .or_else(|| app.help.page.clone())
        .unwrap_or_else(|| "Help".to_string());
    let (can_back, can_forward) = (app.help.can_go(true), app.help.can_go(false));

    div()
        .h(px(38.))
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .border_b_1()
        .border_color(theme::border())
        .child(
            icon_button(
                "help-back",
                IconName::ArrowLeft,
                false,
                cx.listener(|this, _, _, cx| this.step_help(true, cx)),
            )
            .h_full()
            // A step nobody can take is dimmed rather than removed: the pair has to keep its
            // place, or the header's controls shuffle every time a page is opened.
            .when(!can_back, |this| this.opacity(0.4))
            .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("Back").build(window, cx)),
        )
        .child(
            icon_button(
                "help-forward",
                IconName::ArrowRight,
                false,
                cx.listener(|this, _, _, cx| this.step_help(false, cx)),
            )
            .h_full()
            .when(!can_forward, |this| this.opacity(0.4))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Forward").build(window, cx)
            }),
        )
        .child(
            icon_button(
                "help-contents",
                UbiqIcon::ViewTree,
                app.help.contents_open,
                cx.listener(|this, _, _, cx| this.toggle_help_contents(cx)),
            )
            .h_full()
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Contents").build(window, cx)
            }),
        )
        .child(div().flex_1().min_w(px(0.)).pl_2().pr_3().child(elided(
            "help-title",
            title,
            theme::text(),
            theme::font(theme::Family::Chrome, theme::Role::Body),
        )))
        .child(
            div()
                .id("help-follow")
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .gap_1p5()
                .pr_2()
                .child(kit::check_box(
                    "help-follow-box",
                    app.help.follow,
                    cx.listener(|this, _, _, cx| this.toggle_help_follow(cx)),
                ))
                .child(
                    kit::mono("Follow mode".to_string(), theme::text_muted())
                        .text_size(theme::font(theme::Family::Chrome, theme::Role::Meta)),
                )
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(
                        "Switch the page automatically when the mode or window changes",
                    )
                    .build(window, cx)
                }),
        )
}

/// The contents tree: the catalogue's nav order, indented by each page's depth, with a twisty on
/// any row the next one is deeper than.
fn contents(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let size = f32::from(theme::font(theme::Family::Chrome, theme::Role::Body));
    let current = app.help.page.clone();
    let catalog = app.help.state.catalog().cloned().unwrap_or_default();

    let rows: Vec<AnyElement> = app
        .help
        .nav_rows()
        .into_iter()
        .map(|(id, depth, has_children)| {
            let page = catalog.page(&id);
            let label = page
                .map(|page| page.title.clone())
                .unwrap_or_else(|| id.clone());
            // A planned page is listed so the shape of the manual is honest, and greyed so nobody
            // reads the listing as a promise that there is something behind it.
            let planned = page.is_some_and(|page| page.status == HelpStatus::Planned);
            let colour = if planned {
                theme::text_faint()
            } else {
                theme::text_muted()
            };
            let selected = current.as_deref() == Some(id.as_str());
            let folded = app.help.folded.contains(&id);
            let open = id.clone();

            div()
                .id(eid("help-nav", &id))
                .h(px(row_height(size)))
                .pl(px(row_indent(size) * f32::from(depth)))
                .pr_2()
                .flex()
                .items_center()
                .gap_1()
                .when(selected, |this| this.bg(theme::selected()))
                .hover(|this| this.bg(theme::hover()))
                .when(has_children, |this| {
                    let branch = id.clone();
                    let fold = ui::handler(&cx.entity(), move |this, _, cx| {
                        this.toggle_help_fold(&branch, cx)
                    });
                    this.child(twisty(
                        eid("help-fold", &id),
                        !folded,
                        move |_, window, cx| fold(window, cx),
                    ))
                })
                .when(!has_children, |this| this.child(div().w(px(16.))))
                .child(elided(eid("help-nav-label", &id), label, colour, px(size)))
                .on_click({
                    let go = ui::handler(&cx.entity(), move |this, window, cx| {
                        this.open_help_page(&open, window, cx)
                    });
                    move |_, window: &mut Window, cx: &mut App| go(window, cx)
                })
                .into_any_element()
        })
        .collect();

    div()
        .id("help-contents")
        .w(px(CONTENTS_WIDTH))
        .flex_none()
        .h_full()
        .overflow_y_scroll()
        .border_r_1()
        .border_color(theme::border())
        .children(rows)
}

/// The page itself, or the honest thing to say where there is no page.
fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    match &app.help.state {
        // The ask is in flight. Nothing to say yet, and nothing to apologise for.
        HelpState::Unknown => stub(app, "help.pending", cx),
        HelpState::Unavailable { .. } => stub(app, "help.unavailable", cx),
        HelpState::Ready { catalog, .. } => ready(app, catalog, cx),
    }
}

/// The built-in page, drawn by the same renderer everything else is.
fn stub(app: &AppState, key: &str, cx: &mut Context<AppState>) -> AnyElement {
    page_body(app, key, STUB, None, cx)
}

fn ready(app: &AppState, catalog: &HelpCatalog, cx: &mut Context<AppState>) -> AnyElement {
    let Some(id) = app.help.page.clone() else {
        return stub(app, "help.pending", cx);
    };
    let page = catalog.page(&id);
    let source = app.help.sources.get(&id).and_then(Option::as_deref);

    match (page, source) {
        // Written, and here. A draft says so above the body rather than in the tree, because the
        // caveat is about what you are reading, not about whether to open it.
        (Some(page), Some(source)) if page.status != HelpStatus::Planned => {
            let note = (page.status == HelpStatus::Draft).then_some(
                "This page is a draft: it is behind the build you are running in places.",
            );
            page_body(app, &id, source, note, cx)
        }
        // Listed and not written — a planned page, or one whose file the bundle does not hold.
        // The link that led here still highlighted; what is missing is said, with the id, so it
        // can be reported.
        _ => unwritten(&id),
    }
}

/// What a page that does not exist yet reads as.
fn unwritten(id: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .p_5()
        .gap_2()
        .child(
            div()
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .text_color(theme::text_muted())
                .child("This page has not been written yet."),
        )
        .child(kit::mono(id.to_string(), theme::text_faint()))
        .into_any_element()
}

/// A markdown body, with the panel's own link policy on it.
fn page_body(
    app: &AppState,
    key: &str,
    source: &str,
    note: Option<&'static str>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let base = app
        .help
        .current()
        .map(|page| folder_of(&page.path))
        .unwrap_or_default();
    let document = viewer::markdown::render_linked(
        app,
        key,
        source,
        None,
        false,
        follow(cx.entity(), base),
        cx,
    );
    let mut column = div().flex().flex_col().flex_1().min_h(px(0.));
    if let Some(note) = note {
        column = column.child(draft_note(note));
    }
    column.child(document).into_any_element()
}

/// The strip above a draft page. A status colour rather than wording alone, per the theme's rule.
fn draft_note(note: &'static str) -> impl IntoElement {
    kit::slab(theme::warning())
        .px_3()
        .py_1p5()
        .bg(theme::warning_soft())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::Info).with_size(Size::Small))
                .child(
                    div()
                        .text_size(theme::font(theme::Family::Chrome, theme::Role::Meta))
                        .text_color(theme::text_muted())
                        .child(note),
                ),
        )
}

/// A page's folder, which a relative link is resolved against. The empty string for a page at the
/// root, which is what `resolve` below expects.
fn folder_of(path: &str) -> String {
    match path.rfind('/') {
        Some(cut) => path[..cut].to_string(),
        None => String::new(),
    }
}

/// What a clicked link in a help page does.
///
/// **Id first, path second.** An id is the documented way to name a page and survives the page
/// moving; a path is the form that works and breaks. Everything the catalogue cannot answer falls
/// through to the window's own link handler, which is what opens a `ubiq://` place and hands the
/// web to the operating system.
fn follow(
    app: Entity<AppState>,
    base: String,
) -> impl Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync + 'static {
    let elsewhere = ui::on_link(app.clone(), None);
    move |target, event, window, cx| {
        let text = target.trim();
        let (path, _anchor) = match text.find('#') {
            Some(cut) => (&text[..cut], Some(&text[cut + 1..])),
            None => (text, None),
        };
        // A bare `#heading` is a jump inside the page being read. The renderer exposes no
        // scroll-to-anchor, so it is a no-op here rather than a navigation away from the page.
        if path.is_empty() {
            return;
        }
        let hit = app
            .read(cx)
            .help
            .state
            .catalog()
            .and_then(|catalog| resolve(catalog, &base, path));
        match hit {
            Some(id) => app.update(cx, |this, cx| this.open_help_page(&id, window, cx)),
            None => elsewhere(target, event, window, cx),
        }
    }
}

/// The page a link target names, if it names one: an id, a redirect from an id, a path relative to
/// the page's folder, or a redirect from that path.
fn resolve(catalog: &HelpCatalog, base: &str, target: &str) -> Option<String> {
    if let Some(page) = catalog.page(target) {
        return Some(page.id.clone());
    }
    let path = join(base, target)?;
    if let Some(page) = catalog.pages.iter().find(|page| page.path == path) {
        return Some(page.id.clone());
    }
    catalog
        .redirects
        .get(&path)
        .and_then(|id| catalog.page(id))
        .map(|page| page.id.clone())
}

/// `base` plus a relative target, `.` and `..` resolved. `None` for anything that climbs out of
/// the content root, which names no page by construction.
fn join(base: &str, target: &str) -> Option<String> {
    let mut stack: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            part => stack.push(part),
        }
    }
    (!stack.is_empty()).then(|| stack.join("/"))
}
