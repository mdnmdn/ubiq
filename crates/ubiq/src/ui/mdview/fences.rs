//! The blocks the markdown view does not draw as text: a ```` ```mermaid ```` or
//! ```` ```excalidraw ```` fence, and a standalone image.
//!
//! **One renderer per format.** A fence is drawn by the same code the diagram panel and the
//! TextView preview use (`ui/viewer/diagram.rs`, `ui/viewer/scene.rs`), and an image by the
//! preview's own image block (`ui/viewer/markdown.rs`). This module only decides which, and keys
//! the diagram cache the way those callers do.
//!
//! **The key is the mdast body.** `AppState`'s diagram cache is keyed on the fence's source string,
//! and every other caller derives that string from the `markdown` crate's AST (`code.value`). The
//! engine's `BlockKind::Code::body` differs in two ways: it keeps the final line ending, which the
//! AST drops, and it has every line ending as `\n` where the AST keeps a CRLF document's `\r\n`.
//! [`fence_key`] undoes both. A difference of one byte is a
//! cache miss, which is a fence that draws an ellipsis forever; the test below pins it.

use gpui::AnyElement;
use ubiq_md::{Block, BlockKind, Document};

use crate::app::AppState;
use crate::ui::viewer::markdown::{ImageBlock, render_image_block};
use crate::ui::viewer::{diagram, scene};

/// Which renderer a fence's info string names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mermaid,
    Excalidraw,
}

impl Format {
    pub fn of(language: Option<&str>) -> Option<Format> {
        match language.map(str::trim) {
            Some("mermaid") => Some(Format::Mermaid),
            Some("excalidraw") => Some(Format::Excalidraw),
            _ => None,
        }
    }
}

/// A fence body as the diagram cache knows it — the mdast `code.value` of the same fence: no final
/// line ending, and the document's own line endings (`crlf` for a document written with `\r\n`).
pub fn fence_key(body: &str, crlf: bool) -> String {
    let body = body.strip_suffix('\n').unwrap_or(body);
    if crlf {
        body.replace('\n', "\r\n")
    } else {
        body.to_string()
    }
}

/// Whether a document is written with CRLF line endings — what [`fence_key`] needs to know.
pub fn is_crlf(source: &str) -> bool {
    source.contains("\r\n")
}

/// The picture a fenced code block draws as, or `None` for a fence that is just code. `source` is
/// the whole document's, for its line endings.
pub fn fence(language: Option<&str>, body: &str, source: &str) -> Option<AnyElement> {
    let format = Format::of(language)?;
    let key = fence_key(body, is_crlf(source));
    Some(match format {
        Format::Mermaid => diagram::drawn(&key),
        Format::Excalidraw => scene::render(&key),
    })
}

/// A standalone image, capped at the published measure, with the preview's own treatment.
pub fn image(dest: &str, alt: &str) -> AnyElement {
    render_image_block(&ImageBlock {
        url: dest.to_string(),
        alt: alt.to_string(),
    })
}

/// Resolve every mermaid fence in `doc` against the window's diagram cache, and publish the
/// measure pictures scale down to — what the rows read back while they draw, since a `list()` row
/// is handed no `AppState`. Called by the view before it returns its list each frame; it replaces
/// `ui/viewer/markdown.rs`'s `scan_and_publish` for the views that draw through this module.
pub fn publish(app: &AppState, doc: &Document, measure: Option<f32>) {
    diagram::publish_measure(measure);
    for (format, key) in fences(doc) {
        if format == Format::Mermaid {
            diagram::publish(app, &key);
        }
    }
}

/// Every diagram fence in a document, nested ones included, in document order, keyed.
pub fn fences(doc: &Document) -> Vec<(Format, String)> {
    let mut out = Vec::new();
    collect(&doc.blocks, is_crlf(&doc.source), &mut out);
    out
}

fn collect(blocks: &[Block], crlf: bool, out: &mut Vec<(Format, String)>) {
    for block in blocks {
        match &block.kind {
            BlockKind::Code { language, body, .. } => {
                if let Some(format) = Format::of(language.as_deref()) {
                    out.push((format, fence_key(body, crlf)));
                }
            }
            BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                collect(blocks, crlf, out)
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    collect(&item.blocks, crlf, out);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::text::markdown_ast;

    /// Every diagram fence the TextView preview's parse sees, with its `code.value` — the key the
    /// rest of Ubiq files the diagram under.
    fn ast_keys(source: &str) -> Vec<String> {
        fn walk(node: &markdown_ast::Node, out: &mut Vec<String>) {
            if let markdown_ast::Node::Code(code) = node
                && Format::of(code.lang.as_deref()).is_some()
            {
                out.push(code.value.clone());
            }
            if let Some(children) = node.children() {
                for child in children {
                    walk(child, out);
                }
            }
        }
        let ast = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).expect("parses");
        let mut out = Vec::new();
        walk(&ast, &mut out);
        out
    }

    /// The regression the TextView path already lived through: the key published from the engine's
    /// fence body and the key the rest of Ubiq looks up are one string, byte for byte.
    #[test]
    fn the_fence_key_is_the_mdast_body() {
        let documents = [
            (
                "plain",
                "# T\n\n```mermaid\ngraph TD;\nA-->B;\n```\n\ntail\n",
            ),
            (
                "trailing blank line",
                "```mermaid\ngraph TD;\nA-->B;\n\n```\n",
            ),
            ("no newline at eof", "```mermaid\ngraph TD;\nA-->B;\n```"),
            (
                "indented in a list",
                "- item\n\n  ```mermaid\n  graph TD;\n  A-->B;\n  ```\n",
            ),
            ("in a quote", "> ```mermaid\n> graph TD;\n> A-->B;\n> ```\n"),
            (
                "crlf",
                "# T\r\n\r\n```mermaid\r\ngraph TD;\r\nA-->B;\r\n```\r\n",
            ),
            (
                "two fences and an excalidraw",
                "```mermaid\nA\n```\n\ntext\n\n```excalidraw\n{}\n```\n\n```mermaid\nB\n```\n",
            ),
            ("not a diagram", "```rust\nfn main() {}\n```\n"),
        ];
        for (name, source) in documents {
            let ours: Vec<String> = fences(&ubiq_md::parse(source))
                .into_iter()
                .map(|(_, key)| key)
                .collect();
            assert_eq!(ours, ast_keys(source), "{name}");
        }
    }

    #[test]
    fn only_diagram_tags_are_fences() {
        assert_eq!(Format::of(Some("mermaid")), Some(Format::Mermaid));
        assert_eq!(Format::of(Some(" excalidraw ")), Some(Format::Excalidraw));
        assert_eq!(Format::of(Some("rust")), None);
        assert_eq!(Format::of(None), None);
    }
}
