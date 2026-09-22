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

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use gpui::{AnyElement, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::text::{MarkdownExtensions, MarkdownNode, TextView, markdown_ast};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::{disclosure, mono};
use crate::ui::{eid, eid2, on_link};

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

impl Fence {
    /// The one place a code block becomes a fence.
    ///
    /// **Both sides call this**: the scan that publishes a diagram's resolution and the block
    /// parser that later asks for it. The map between them is keyed on `source`, so the two
    /// strings have to be the same byte for byte — which they are only because neither side
    /// derives its own.
    fn of(code: &markdown_ast::Code) -> Option<Self> {
        Some(Fence {
            format: Format::of(code.lang.as_deref().unwrap_or(""))?,
            source: code.value.clone(),
        })
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

/// What one document's fence scan and frontmatter split produced, kept until its source changes.
///
/// Both are a full pass over the document plus an allocation, and `TextView` already skips its own
/// work when the string it is handed is unchanged — so redoing this every frame regardless was pure
/// waste ahead of a guard that was going to short-circuit anyway. The body is a `SharedString`
/// rather than a `String`: an unchanged frame hands `TextView` the same reference-counted buffer, a
/// clone that costs nothing rather than a fresh copy of the whole document.
struct Scanned {
    len: usize,
    hash: u64,
    fences: Vec<Fence>,
    frontmatter: Option<String>,
    body: SharedString,
}

thread_local! {
    /// One entry per open document. Never evicted, like the diagram resolution cache beside it —
    /// a tab's worth of Markdown text is small, and a session opens few enough documents that this
    /// never grows into a problem worth a lifecycle to solve.
    static SCAN_CACHE: RefCell<HashMap<String, Scanned>> = RefCell::new(HashMap::new());
}

/// A cheap stand-in for the document's identity: exact equality would cost as much as the scan it
/// is meant to avoid paying for, so length plus a hash is the fingerprint instead.
fn fingerprint(source: &str) -> (usize, u64) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    (source.len(), hasher.finish())
}

/// Re-scan the document only when it changed since the last frame, and resolve every Mermaid fence
/// found against the window's cache either way — that half is a hashmap lookup, not a scan, and has
/// to run every frame so a diagram still in `Pending` catches up once it lands.
fn scan_and_publish(app: &AppState, key: &str, source: &str) -> (Option<String>, SharedString) {
    let (len, hash) = fingerprint(source);
    SCAN_CACHE.with_borrow_mut(|cache| {
        let stale =
            !matches!(cache.get(key), Some(cached) if cached.len == len && cached.hash == hash);
        if stale {
            let (fm, body) = split_frontmatter(source);
            cache.insert(
                key.to_string(),
                Scanned {
                    len,
                    hash,
                    // The body, not the source: the body is what the text view parses, and a
                    // fence has to be scanned out of the same string to be keyed the same way.
                    fences: fences(body),
                    frontmatter: fm.map(str::to_string),
                    body: SharedString::from(body.to_string()),
                },
            );
        }
        let cached = cache.get(key).expect("just inserted, or already fresh");
        for fence in &cached.fences {
            if fence.format == Format::Mermaid {
                super::diagram::publish(app, &fence.source);
            }
        }
        (cached.frontmatter.clone(), cached.body.clone())
    })
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
    frontmatter_open: bool,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let follow = on_link(
        cx.entity(),
        Some(crate::state::editor::from_tab_key(key).0.into()),
    );
    render_linked(app, key, source, frontmatter_open, follow, cx)
}

/// The same document, with a caller's own answer to a clicked link.
///
/// One renderer, two link policies. A file's link is resolved against the project it is read in,
/// which is [`render`] above; a help page's is resolved against the catalogue first — an id is the
/// documented way to name a page and resolves before any path does — which is `ui::help`. Both
/// draw the same markdown, the same fences and the same diagrams, because there is one renderer
/// and only the seam below differs.
pub fn render_linked(
    app: &AppState,
    key: &str,
    source: &str,
    frontmatter_open: bool,
    follow: impl Fn(&SharedString, &gpui::ClickEvent, &mut gpui::Window, &mut gpui::App)
    + Send
    + Sync
    + 'static,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let (frontmatter, body) = scan_and_publish(app, key, source);

    // Keyed on the settled point size as well as the file: the text view keeps the height it
    // measured each block at and only reconsiders when its width changes, so a zoom needs a new
    // state to reflow at all. `md_reflow` moves half a second after the last zoom, which is what
    // keeps a held key from rebuilding the document once per point.
    let document = TextView::markdown(eid2("md", key, app.md_reflow), body)
        .markdown_extensions(extensions().clone())
        .on_link_click(follow)
        .p_5()
        .text_size(theme::font(theme::Family::Content, theme::Role::Body))
        .scrollable(true)
        .selectable(true);

    let Some(raw_yaml) = frontmatter else {
        return document.into_any_element();
    };

    // The bar keeps its own height and the document takes what is left: a scrollable text view
    // fills the box it is given, so the box has to be bounded or it scrolls nothing.
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
        .child(frontmatter_bar(key, &raw_yaml, frontmatter_open, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(document),
        )
        .into_any_element()
}

/// One block's own markdown, drawn standalone rather than as part of a whole document.
///
/// The plan annotation panel's own unit (`ui/plan.rs`): a task's plan arrives from the host
/// already split into [`ubiq_proto::plan::PlanBlock`]s, and each is rendered through this rather
/// than through [`render`] so that a block can be its own clickable, hit-testable element — the
/// granularity `_docs/wip/planning-system.md` (staging slice 4) settles for, in place of a
/// character-range selection model. It shares [`render`]'s fence support: a block that is itself a
/// ` ```mermaid ` fence still resolves and draws through `super::diagram`, keyed on its own id so
/// it does not collide with the whole document's scan.
///
/// Not scrollable and not padded like a full document — a block is short by construction, and it
/// sits inside a container the caller already gives padding and a click target.
pub fn render_block(app: &AppState, key: &str, text: &str) -> AnyElement {
    let (_, body) = scan_and_publish(app, key, text);
    TextView::markdown(eid("md-block", key), body)
        .markdown_extensions(extensions().clone())
        .text_size(theme::font(theme::Family::Content, theme::Role::Body))
        .selectable(true)
        .into_any_element()
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
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let key = key.to_string();
    let summary = mono(frontmatter_summary(raw_yaml), theme::text_faint())
        .text_size(theme::font(theme::Family::Content, theme::Role::Dense));

    div()
        .flex()
        .flex_col()
        .flex_none()
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
                    mono(raw_yaml.to_string(), theme::text_muted())
                        .text_size(theme::font(theme::Family::Content, theme::Role::Dense)),
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
                let fence = Fence::of(code)?;
                // The text representation is the source, so copying a document out of the view
                // still carries what the diagram was written as.
                let text = fence.source.clone();
                Some(MarkdownNode::new(FENCE, fence).text(text))
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
/// **Parsed, not scanned.** This runs ahead of the text view's own parse, in this frame, because a
/// fence's picture has to be resolved before the block renderer that draws it asks for one — but it
/// runs the *same* parser over the *same* string, so the body it publishes is the body the block
/// parser will be handed. A hand-rolled scanner stood here and reconstructed each body line by
/// line: it ended every fence with a newline the AST does not carry, kept the indentation the AST
/// strips, and kept the `\r` of a CRLF line the AST drops — so the key it published never matched
/// the key the renderer looked up, and every fence drew an ellipsis forever.
///
/// The options are [`markdown::ParseOptions::gfm`] because that is what `MarkdownExtensions`
/// resolves to for a view with no MDX enabled, which is every view here.
fn fences(source: &str) -> Vec<Fence> {
    let Ok(ast) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    collect(&ast, &mut found);
    found
}

/// Every fence in a subtree, in document order.
fn collect(node: &markdown_ast::Node, found: &mut Vec<Fence>) {
    if let markdown_ast::Node::Code(code) = node
        && let Some(fence) = Fence::of(code)
    {
        found.push(fence);
    }
    if let Some(children) = node.children() {
        for child in children {
            collect(child, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `Code` node the block parser would be offered, as the view's own parse produces them.
    fn ast_code_values(source: &str) -> Vec<(Option<String>, String)> {
        fn walk(node: &markdown_ast::Node, out: &mut Vec<(Option<String>, String)>) {
            if let markdown_ast::Node::Code(code) = node {
                out.push((code.lang.clone(), code.value.clone()));
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

    /// The regression: the published key and the looked-up key are one string.
    ///
    /// Each of these once differed — a trailing newline, an indent, a `\r` — and a difference of
    /// one byte is a cache miss, which is a fence that draws nothing.
    #[test]
    fn published_fence_body_is_the_ast_body() {
        let documents = [
            (
                "plain",
                "# T\n\n```mermaid\ngraph TD;\nA-->B;\n```\n\ntail\n",
            ),
            (
                "trailing blank line",
                "```mermaid\ngraph TD;\nA-->B;\n\n```\n",
            ),
            (
                "indented",
                "- item\n\n  ```mermaid\n  graph TD;\n  A-->B;\n  ```\n",
            ),
            (
                "crlf",
                "# T\r\n\r\n```mermaid\r\ngraph TD;\r\nA-->B;\r\n```\r\n",
            ),
            (
                "two fences",
                "```mermaid\nA\n```\n\ntext\n\n```mermaid\nB\n```\n",
            ),
        ];
        for (name, source) in documents {
            let scanned: Vec<String> = fences(source).iter().map(|f| f.source.clone()).collect();
            let parsed: Vec<String> = ast_code_values(source)
                .into_iter()
                .filter(|(lang, _)| Format::of(lang.as_deref().unwrap_or("")).is_some())
                .map(|(_, value)| value)
                .collect();
            assert_eq!(
                scanned, parsed,
                "{name}: published {scanned:?} vs AST {parsed:?}"
            );
        }
    }

    #[test]
    fn scanner_finds_tagged_fences_only() {
        let source = concat!(
            "```rust\nfn main() {}\n```\n\n",
            "```mermaid\ngraph TD;\nA-->B;\n```\n\n",
            "```excalidraw\n{}\n```\n\n",
            "```\nplain\n```\n",
        );
        let found = fences(source);
        assert_eq!(found.len(), 2);
        assert!(found[0].format == Format::Mermaid);
        assert_eq!(found[0].source, "graph TD;\nA-->B;");
        assert!(found[1].format == Format::Excalidraw);
        assert_eq!(found[1].source, "{}");
    }

    /// A fence nested in a list or a blockquote is still a fence, and the AST reaches it.
    #[test]
    fn scanner_reaches_nested_fences() {
        let source = "- item\n\n  ```mermaid\n  graph TD;\n  A-->B;\n  ```\n\n> ```mermaid\n> flowchart LR\n> ```\n";
        let found = fences(source);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].source, "graph TD;\nA-->B;");
        assert_eq!(found[1].source, "flowchart LR");
    }

    /// A fence that would be drawn is published out of the *body*, so frontmatter never shifts it.
    #[test]
    fn frontmatter_is_split_before_the_fences_are_read() {
        let source = "---\ntitle: x\n---\n\n```mermaid\ngraph TD;\nA-->B;\n```\n";
        let (fm, body) = split_frontmatter(source);
        assert_eq!(fm, Some("title: x"));
        let found = fences(body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "graph TD;\nA-->B;");
    }

    /// A tag that names no renderer is not a fence, whatever its body says.
    #[test]
    fn untagged_and_unknown_fences_are_not_diagrams() {
        assert!(fences("```\ngraph TD;\n```\n").is_empty());
        assert!(fences("```mermaidx\ngraph TD;\n```\n").is_empty());
        assert_eq!(fences("``` mermaid \ngraph TD;\n```\n").len(), 1);
    }
}
