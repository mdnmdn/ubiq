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

use gpui::{AnyElement, IntoElement, Styled};
use gpui_component::text::{MarkdownExtensions, MarkdownNode, TextView, markdown_ast};

use crate::app::AppState;
use crate::theme;
use crate::ui::eid;

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
pub fn render(app: &AppState, key: &str, source: &str) -> AnyElement {
    for fence in fences(source) {
        if fence.format == Format::Mermaid {
            super::diagram::publish(app, &fence.source);
        }
    }

    TextView::markdown(eid("md", key), source.to_string())
        .markdown_extensions(extensions().clone())
        .p_5()
        .scrollable(true)
        .selectable(true)
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
