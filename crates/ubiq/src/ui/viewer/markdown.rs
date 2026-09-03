//! A Markdown document, drawn.
//!
//! The component library's text view does the work — GFM, tables, task lists, inline images — and
//! the source half is the file's own buffer rather than a copy of it, so a toggle between the two
//! keeps the undo history a copy would throw away.
//!
//! What this module adds is **fences**: a ```` ```mermaid ```` or ```` ```excalidraw ```` block is
//! drawn by the same renderer the panel uses, so there is one renderer per format and two call
//! sites for it. The hook is the text view's own — a block parser runs before the built-in
//! code-block conversion, and a block renderer draws what it produced.

use std::sync::OnceLock;

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use gpui_component::text::{MarkdownExtensions, MarkdownNode, TextView, markdown_ast};

use crate::app::AppState;
use crate::theme;
use crate::ui::eid;
use crate::ui::kit::{disclosure, mono};

/// The name the fence parser gives its nodes and the renderer answers to.
const FENCE: &str = "ubiq-diagram-fence";

/// Which renderer a fence's tag names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Mermaid,
    Excalidraw,
}

impl Format {
    fn of(tag: &str) -> Option<Self> {
        match tag.trim() {
            "mermaid" => Some(Format::Mermaid),
            "excalidraw" => Some(Format::Excalidraw),
            _ => None,
        }
    }
}

/// What a fence carries from the parse to the drawing.
///
/// Typed data rather than an element, because **the parser runs on a background task** and is
/// handed neither a window nor an application.
#[derive(Clone)]
struct Fence {
    format: Format,
    source: String,
}

/// The document.
///
/// Every Mermaid fence in it is resolved against the window's cache first, because the block
/// renderer that draws one is handed no way to do it itself. A fence nobody has asked for yet is
/// asked for here, so a document holding several fills in as the answers arrive.
pub fn render(
    app: &AppState,
    key: &str,
    source: &str,
    font_size: Option<f32>,
    frontmatter_open: bool,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    for fence in fences(source) {
        if fence.format == Format::Mermaid {
            super::diagram::publish(app, &fence.source);
        }
    }

    let size = font_size.unwrap_or(theme::EDITOR_FONT_SIZE);
    let (fm, body) = split_frontmatter(source);

    if let Some(raw_yaml) = fm {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_y_hidden()
            .child(frontmatter_bar(key, raw_yaml, frontmatter_open, size, cx))
            .child(
                TextView::markdown(eid("md", key), body.to_string())
                    .markdown_extensions(extensions().clone())
                    .p_5()
                    .text_size(px(size))
                    .scrollable(false)
                    .selectable(true),
            )
            .into_any_element()
    } else {
        TextView::markdown(eid("md", key), body.to_string())
            .markdown_extensions(extensions().clone())
            .p_5()
            .text_size(px(size))
            .scrollable(true)
            .selectable(true)
            .into_any_element()
    }
}

/// Split YAML frontmatter from the Markdown body.
///
/// Returns `(Some(yaml), body)` when the source opens with `---\n`, or `(None, source)` when it
/// does not. The body is clean — the parser never sees the opening or closing delimiters.
fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
    let trimmed = source.trim_start();
    let Some(after_open) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    else {
        return (None, source);
    };

    if let Some(end) = after_open.find("\n---") {
        let fm = &after_open[..end];
        let rest = &after_open[end + 4..];
        let body = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix("\n"))
            .unwrap_or(rest);
        (Some(fm), body)
    } else {
        (None, source)
    }
}

/// A one-line summary of frontmatter fields for the collapsed disclosure bar.
fn frontmatter_summary(raw_yaml: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for line in raw_yaml.lines() {
        if let Some((key, _)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty() && parts.len() < 3 {
                parts.push(key);
            }
        }
    }
    if parts.is_empty() {
        format!("{} lines", raw_yaml.lines().count())
    } else {
        parts.join(" \u{00b7} ")
    }
}

/// A collapsible monospaced block that sits at the head of a Markdown preview when the document
/// carries YAML frontmatter.
fn frontmatter_bar(
    key: &str,
    raw_yaml: &str,
    open: bool,
    font_size: f32,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let key = key.to_string();
    let summary =
        mono(frontmatter_summary(raw_yaml), theme::text_faint()).text_size(px(font_size - 1.5));

    div()
        .flex()
        .flex_col()
        .child(disclosure(
            eid("md-fm", &key),
            "Frontmatter",
            summary,
            open,
            cx.listener(move |this, _, _, cx| this.toggle_frontmatter(&key, cx)),
        ))
        .children(open.then(|| {
            div()
                .p_3()
                .bg(theme::pane_bg())
                .border_b_1()
                .border_color(theme::border())
                .child(
                    mono(raw_yaml.to_string(), theme::text_muted()).text_size(px(font_size - 1.0)),
                )
        }))
        .into_any_element()
}

/// The fence hooks, built once for the process.
///
/// **Once, because the registry carries a revision the text view re-parses on.** Building it per
/// frame would give every document a new revision every frame, and each re-parse would land as a
/// notification that asked for the next one.
fn extensions() -> &'static MarkdownExtensions {
    static EXTENSIONS: OnceLock<MarkdownExtensions> = OnceLock::new();
    EXTENSIONS.get_or_init(|| {
        MarkdownExtensions::default()
            .block_parser(|node, _| {
                let markdown_ast::Node::Code(code) = node else {
                    return None;
                };
                let format = Format::of(code.lang.as_deref().unwrap_or(""))?;
                let fence = Fence {
                    format,
                    source: code.value.clone(),
                };
                // The text representation is the source, so copying a document out of the view
                // still carries what the diagram was written as.
                Some(MarkdownNode::new(FENCE, fence).text(code.value.clone()))
            })
            .block_renderer(FENCE, |node, _, _| {
                let Some(fence) = node.data::<Fence>() else {
                    return super::note("Not a fence this build draws", theme::text_faint());
                };
                match fence.format {
                    Format::Mermaid => super::diagram::drawn(&fence.source),
                    Format::Excalidraw => super::scene::render(&fence.source),
                }
            })
    })
}

/// The fenced blocks a document holds that a diagram renderer draws.
///
/// Scanned rather than parsed: the text view's own parse runs on a background task and the answer
/// is needed in this frame, and all this needs of a fence is its tag and its text. Every fence is
/// tracked so that a diagram tag inside a code block is not mistaken for one.
fn fences(source: &str) -> Vec<Fence> {
    let mut found = Vec::new();
    let mut open: Option<(Option<Format>, String)> = None;

    for line in source.lines() {
        let trimmed = line.trim_start();
        match open.take() {
            None => {
                if let Some(tag) = trimmed.strip_prefix("```") {
                    open = Some((Format::of(tag), String::new()));
                }
            }
            Some((format, mut body)) => {
                if trimmed.starts_with("```") {
                    if let Some(format) = format {
                        found.push(Fence {
                            format,
                            source: body,
                        });
                    }
                } else {
                    body.push_str(line);
                    body.push('\n');
                    open = Some((format, body));
                }
            }
        }
    }

    found
}
