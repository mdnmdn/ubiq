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

use gpui::{
    AnyElement, HighlightStyle, ImageSource, InteractiveElement, IntoElement, Overflow,
    ParentElement, Pixels, Resource, SharedString, SharedUri, StyleRefinement, Styled, div, img,
    px, relative,
};
use gpui_component::text::{
    MarkdownExtensions, MarkdownNode, TextView, TextViewStyle, markdown_ast,
};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::{disclosure, mono};
use crate::ui::{eid, eid2, on_link};

/// The name the fence parser gives its nodes and the renderer answers to.
const FENCE: &str = "ubiq-diagram-fence";

/// The name the standalone-image parser gives its nodes (T-185) — a paragraph holding nothing but
/// one `![alt](url)`. Its own block, rather than left to the text view's built-in inline image,
/// because only a block this module owns can carry the reading-measure cap and the zoom button —
/// the same treatment a fenced diagram gets, and for the same reason: T-185 asks for both.
const IMAGE_BLOCK: &str = "ubiq-image-block";

/// What the standalone-image parser carries from the parse to the drawing: just enough to build
/// the same `img(...)` element the library's own inline image would, plus a title for the zoom
/// modal.
#[derive(Clone)]
pub(crate) struct ImageBlock {
    pub(crate) url: String,
    pub(crate) alt: String,
}

impl ImageBlock {
    /// A paragraph that is nothing but one image is a block of its own; a paragraph carrying an
    /// image beside any other inline content — prose before or after it, a second image — is left
    /// to the text view's own inline rendering, which draws it as part of the running text it
    /// actually is. Read off the AST directly rather than off the rendered text.
    fn of(node: &markdown_ast::Node) -> Option<Self> {
        let markdown_ast::Node::Paragraph(paragraph) = node else {
            return None;
        };
        let [markdown_ast::Node::Image(image)] = paragraph.children.as_slice() else {
            return None;
        };
        Some(ImageBlock {
            url: image.url.clone(),
            alt: image.alt.clone(),
        })
    }
}

/// The same `img(...)` source the library's own inline image renderer resolves a markdown image
/// URL through (`gpui_base::text::utils::image_source`) — a `SharedUri` unconditionally, not the
/// generic `From<String>` GPUI offers, which reads a non-URI string as an embedded asset rather
/// than a project-relative path and would silently stop a real image from loading. A URI loads
/// through the application's HTTP client, which serves only the help bundle's `file:` URIs
/// (`app::help_images`); any other image draws nothing.
fn image_source(url: &str) -> ImageSource {
    ImageSource::Resource(Resource::Uri(SharedUri::from(url.to_string())))
}

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

/// The typography this preview draws with, from the window's width preset and density —
/// `_docs/inbox/markdown-improvement-proposal.md` §3–§7 (T-116). Returns the [`TextViewStyle`]
/// and the body line height it was built with, since the caller needs the line height again for
/// the top/bottom insets.
///
/// **What this cannot reach.** `TextViewStyle` exposes one vertical-rhythm knob for every
/// non-heading block (`paragraph_gap`) and a `StyleRefinement` each for code blocks and tables —
/// see the longer note in `theme.rs` above `MD_AVG_CHAR_WIDTH_EM`. Two rules from §3 fall in the
/// gap it leaves:
///
/// - **Per-heading-level line height** (H1 1.15, H2 1.2, H3 1.25, distinct from the body's).
///   Headings never call `.line_height(...)` upstream — only `.text_size(...)` — so the body line
///   height set below cascades to them at their own size instead.
/// - **H1's negative tracking** (~-0.02em). GPUI's `Style`/`TextStyle` carry no letter-spacing
///   field at all (`crates/gpui/src/style.rs` — searched, absent), so there is no knob to turn
///   here even per-element.
///
/// Both are reported, not faked: the closest honest approximation is one body leading for prose
/// and headings alike, sized so it reads well at the tightest heading as well as the loosest
/// paragraph — proposal principle 1 loses some of its "leading shrinks as size grows" nuance, but
/// nothing renders wrong.
///
/// **A third thing this cannot reach, added by T-188:** the reading-options popover's four-way
/// text-colour picker (`crate::state::editor::TextShade`) has nowhere to attach either. Unlike
/// `paragraph_gap`/`code_block`/`table`, this `TextViewStyle` (the `ui`-crate facade, not
/// `gpui_base`'s richer one `crate::ui::conversation` reaches through a different path) carries no
/// foreground colour at all — the prose colour is installed once, window-wide, from the active
/// theme (`install_text_view_defaults` in the vendored `crates/ui/src/text/mod.rs`), and nothing
/// `TextView::style(...)` is handed can override it per instance. So `MdReading::char_scale` above
/// reaches the document (`body_size` is multiplied by it before this function ever sees it) and
/// `MdReading::text_shade` does not: the picker, the per-document state and the persistence all
/// landed, but the rendered body still draws in the theme's own colour regardless of which
/// rectangle is picked. Reaching this needs either an upstream `TextViewStyle` foreground hook or
/// a small patch to the vendored copy — a card of its own, not this one.
fn typography(app: &AppState, body: Pixels) -> (TextViewStyle, f32) {
    let line_height = theme::md_body_line_height(
        app.workbench.settings.ui.md_width,
        app.workbench.settings.ui.md_density,
    );

    // Code blocks and tables *do* get their own line height — their `StyleRefinement` is applied
    // on top of the block's own div, after its default `.text_size(...)`, so it can differ from
    // the body's leading where headings cannot.
    //
    // T-134: a code block and a table keep the paragraph's own left edge and content width rather
    // than breaking out to the column's outer frame — the two are read as part of the same prose
    // flow, and a table starting further left than the sentence above it read as misaligned rather
    // than as a deliberate breakout. T-126's fix still holds: a block wider than the column scrolls
    // horizontally within its own frame instead of spilling past the viewport. The codeblock div
    // already carries an element id (`node.rs`'s `("codeblock", ix)`) and `min_w_0`, which is all
    // GPUI's own `overflow: scroll` needs to become interactively scrollable; the table gets the
    // same treatment through `TextViewStyle::table`'s documented opt-in (`node.rs`'s
    // `render_scroll_table`, chosen over the default wrapping layout precisely when this is set).
    let mut code_block = StyleRefinement::default();
    code_block.text.line_height = Some(relative(theme::MD_CODE_LINE_HEIGHT));
    code_block.overflow.x = Some(Overflow::Scroll);

    let mut table_cell = StyleRefinement::default();
    table_cell.text.line_height = Some(relative(theme::MD_CODE_LINE_HEIGHT));
    let mut table = StyleRefinement::default();
    table.overflow.x = Some(Overflow::Scroll);

    // Inline code (§6.1): the chip's padding, corner radius and baseline offset the proposal asks
    // for have no home in `HighlightStyle` — it carries `color`, `font_weight`, `font_style`,
    // `background_color`, `underline`, `strikethrough` and `fade_out` and nothing box-shaped
    // (`crates/gpui/src/style.rs:580-600`), because inline code is drawn as a highlighted text
    // run, not a box, upstream at `crates/base/src/text/node.rs:1466,1588`. A radius would be
    // dropped here regardless — house rule, no radii anywhere in this UI. What *is* reachable is
    // a more contrasted background and a heavier weight, which is what carries the distinction.
    let inline_code = HighlightStyle {
        background_color: Some(theme::accent_soft().into()),
        font_weight: Some(gpui::FontWeight::MEDIUM),
        ..Default::default()
    };

    let mut style = TextViewStyle {
        heading_base_font_size: body,
        ..TextViewStyle::default()
    };
    style = style
        .paragraph_gap(theme::md_paragraph_gap(
            app.workbench.settings.ui.md_density,
        ))
        .heading_font_size(|level, base| base * theme::md_heading_ratio(level))
        .code_block(code_block)
        .table(table)
        .table_cell(table_cell)
        .inline_code(inline_code);
    (style, line_height)
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
    render_linked_scrollable(app, key, source, frontmatter_open, follow, cx)
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
    render_linked_scrollable(app, key, source, frontmatter_open, follow, cx)
}

fn render_linked_scrollable(
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

    // T-188: the reading-options popover's per-document character-size multiplier, over the
    // system font size rather than an absolute point size — `MdReading::char_scale`, `1.0` for a
    // tab (or a caller like `render_linked`'s help page) that never opened the popover at all.
    let char_scale = app
        .reading_file(key, cx)
        .map_or(1.0, |file| file.md_reading.char_scale);
    let body_size = theme::font(theme::Family::Content, theme::Role::Body) * char_scale;
    let (style, line_height) = typography(app, body_size);
    let measure = theme::md_measure_width(app.workbench.settings.ui.md_width, body_size);
    let line_height_px = body_size * line_height;

    // T-185: every fence and every standalone image in this document scales down to fit the same
    // measure the column itself is capped at. A fence's block renderer is handed no `AppState`, so
    // this is a hand-off exactly like `scan_and_publish`'s own `diagram::publish`, and it is
    // published unconditionally so a stale measure from whichever document rendered last in this
    // frame can never bleed into this one.
    super::diagram::publish_measure(measure.map(f32::from));

    // Keyed on the settled point size as well as the file: the text view keeps the height it
    // measured each block at and only reconsiders when its width changes, so a zoom needs a new
    // state to reflow at all. `md_reflow` moves half a second after the last zoom, which is what
    // keeps a held key from rebuilding the document once per point.
    //
    // Horizontal layout (§5.1): `max_w` caps the column at the preset's measure, `mx_auto`
    // centres it whenever the pane has room for that plus two side margins, and shrinks to
    // `w_full` with the same `px(min_margin)` padding when it does not — the classic
    // max-width-plus-auto-margins pattern satisfies the centre-or-left-inset rule as one
    // declaration rather than as a branch on live pane width, which this call site does not have
    // (no `Window`, only `Context<AppState>`).
    let mut document = TextView::markdown(eid2("md", key, app.md_reflow), body)
        .markdown_extensions(extensions().clone())
        .on_link_click(follow)
        .style(style)
        .text_size(body_size)
        .line_height(relative(line_height))
        .w_full()
        .mx_auto()
        .px(theme::md_min_margin())
        .pt(theme::md_top_inset(line_height_px))
        .pb(theme::md_bottom_inset(line_height_px))
        .scrollable(true)
        .selectable(true);
    if let Some(measure) = measure {
        document = document.max_w(measure);
    }

    match frontmatter {
        None => document.into_any_element(),
        // The bar keeps its own height and the document takes what is left: a scrollable text
        // view fills the box it is given, so the box has to be bounded or it scrolls nothing.
        Some(raw_yaml) => div()
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
            .into_any_element(),
    }
}

/// Split YAML frontmatter from the Markdown body.
///
/// Returns `(Some(yaml), body)` when the source opens with `---\n`, or `(None, source)` when it
/// does not. The body is clean — the parser never sees the opening or closing delimiters.
pub(crate) fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
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

/// The monospaced summary that sits at the head of a Markdown preview when the document carries
/// YAML frontmatter. Every caller left on this renderer (help, the docs fixture, the plan
/// surface) draws it closed and holds no per-document state to open it with; a file tab's
/// preview is `ui/mdview`, which draws the front matter as a block of its own.
fn frontmatter_bar(
    key: &str,
    raw_yaml: &str,
    open: bool,
    _cx: &mut gpui::Context<AppState>,
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
            |_, _, _| {},
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
            .block_parser(|node, _| {
                let block = ImageBlock::of(node)?;
                let text = format!("![{}]({})", block.alt, block.url);
                Some(MarkdownNode::new(IMAGE_BLOCK, block).text(text))
            })
            .block_renderer(IMAGE_BLOCK, |node, _, _| {
                let Some(block) = node.data::<ImageBlock>() else {
                    return super::note("Not an image this build draws", theme::text_faint());
                };
                render_image_block(block)
            })
    })
}

/// A standalone markdown image (T-185): the picture, capped at the document's reading measure —
/// never blown up past its own size, only shrunk down when it overruns the column, the same rule
/// `diagram::scale_to_measure` gives a fenced diagram — with the corner zoom button beside it.
///
/// **The zoom modal needs the picture's own pixel size** to seed its pan-and-zoom camera, and a
/// fenced diagram already carries that from the renderer that drew it. A plain image does not:
/// GPUI decodes it lazily behind the same `img(url)` element the library's own inline image already
/// draws, and there is no width or height in hand at parse time to publish one with. So this block
/// gets the resize half of T-185 and not the modal — reading its decoded size back out to raise one
/// is a card of its own.
pub(crate) fn render_image_block(block: &ImageBlock) -> AnyElement {
    let mut picture = img(image_source(&block.url)).flex_none();
    if let Some(measure) = super::diagram::current_measure() {
        picture = picture.max_w(px(measure));
    }
    div()
        .id(eid("md-image", &block.url))
        .w_full()
        .flex()
        .items_start()
        .py_1()
        .child(picture)
        .into_any_element()
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

    /// A paragraph that is nothing but one image is the standalone-image block T-185's resize and
    /// zoom button reach.
    #[test]
    fn a_paragraph_holding_only_an_image_is_the_image_block() {
        let ast = markdown::to_mdast("![a cat](cat.png)\n", &markdown::ParseOptions::gfm())
            .expect("parses");
        let paragraph = &ast.children().expect("root has children")[0];
        let block = ImageBlock::of(paragraph).expect("an image-only paragraph is a block");
        assert_eq!(block.url, "cat.png");
        assert_eq!(block.alt, "a cat");
    }

    /// An image beside any other inline content is left to the text view's own inline rendering —
    /// only a paragraph that is *nothing else* becomes a block.
    #[test]
    fn an_image_beside_prose_is_not_the_image_block() {
        let ast = markdown::to_mdast("look: ![a cat](cat.png)\n", &markdown::ParseOptions::gfm())
            .expect("parses");
        let paragraph = &ast.children().expect("root has children")[0];
        assert!(ImageBlock::of(paragraph).is_none());
    }
}
