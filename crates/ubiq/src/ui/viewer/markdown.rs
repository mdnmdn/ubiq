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
use std::rc::Rc;
use std::sync::OnceLock;

use gpui::{
    AnyElement, HighlightStyle, InteractiveElement, IntoElement, Overflow, ParentElement, Pixels,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, div, px, relative,
};
use gpui_component::scroll::Scrollbar;
use gpui_component::text::{
    MarkdownExtensions, MarkdownNode, TextView, TextViewStyle, markdown_ast,
};

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
    render_linked_scrollable(app, key, source, frontmatter_open, follow, None, cx)
}

/// The same document, scrolled by the caller's own handle rather than the text view's internal
/// one — what the standard viewer's minimap (T-118) needs to jump to a heading: `TextView` keeps
/// its scroll position to itself (`state.rs`'s `scroll_offset` is `pub(super)`), so there is no
/// way to move it from outside. The text view grows to its natural height instead, and an outer
/// scrollable frame carries `scroll`, the same shape `ui/plan.rs`'s empty-document fallback
/// wraps a single `TextView` in.
pub fn render_scrollable(
    app: &AppState,
    key: &str,
    source: &str,
    frontmatter_open: bool,
    scroll: &gpui::ScrollHandle,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let follow = on_link(
        cx.entity(),
        Some(crate::state::editor::from_tab_key(key).0.into()),
    );
    render_linked_scrollable(app, key, source, frontmatter_open, follow, Some(scroll), cx)
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
    render_linked_scrollable(app, key, source, frontmatter_open, follow, None, cx)
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
    scroll: Option<&gpui::ScrollHandle>,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let (frontmatter, body) = scan_and_publish(app, key, source);

    let body_size = theme::font(theme::Family::Content, theme::Role::Body);
    let (style, line_height) = typography(app, body_size);
    let measure = theme::md_measure_width(app.workbench.settings.ui.md_width, body_size);
    let line_height_px = body_size * line_height;

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
        // An external `scroll` owns the position instead: the text view grows to its content's
        // full height and the outer frame below scrolls it, because its own internal scroll
        // offset is `pub(super)` in the component library and cannot be read or moved from here.
        .scrollable(scroll.is_none())
        .selectable(true);
    if let Some(measure) = measure {
        document = document.max_w(measure);
    }

    let content = match frontmatter {
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
    };

    let Some(scroll) = scroll else {
        return content;
    };
    // T-126: the scrollbar is an absolutely-positioned sibling of the scroll area (the
    // `ubiq-ui` rule), sized to the whole pane rather than the centred reading column, so it
    // sits flush at the viewport's own edge and draws through the same `Scrollbar` the rest of
    // the app uses — not TextView's own internal one, which this call site never turns on.
    div()
        .relative()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            div()
                .id(eid("md-scroll", key))
                .size_full()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .child(content),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .child(Scrollbar::vertical(scroll)),
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
    let body_size = theme::font(theme::Family::Content, theme::Role::Body);
    let (style, line_height) = typography(app, body_size);
    TextView::markdown(eid("md-block", key), body)
        .markdown_extensions(extensions().clone())
        .style(style)
        .text_size(body_size)
        .line_height(relative(line_height))
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

/// One heading, positioned proportionally down the document it was found in.
///
/// The standard viewer's minimap (T-118, proposal §8.2's structure strip, reduced to what is
/// reachable here) draws one of these per heading. `fraction` is the heading's own byte offset
/// over the document's total length — the same honest approximation `ui/plan.rs`'s
/// `proportional_fraction` falls back to when there is nothing painted yet to measure against,
/// promoted to the only answer here because the standard viewer draws one `TextView` rather than
/// a block per heading, so there is no per-heading layout to measure in the first place.
pub struct HeadingMark {
    pub level: u8,
    pub fraction: f32,
    /// The heading's own text, with its markup gone — what the header's navigator (T-124) lists.
    /// The minimap has no use for it and pays nothing for it: one string per heading, built on the
    /// same walk.
    pub label: String,
}

/// Every heading in a document, in document order, positioned by character offset.
///
/// Off the shared, cached parse — see [`walks`].
pub fn heading_marks(key: &str, source: &str) -> Rc<Vec<HeadingMark>> {
    walks(key, source).0
}

fn collect_headings(node: &markdown_ast::Node, len: f32, found: &mut Vec<HeadingMark>) {
    if let markdown_ast::Node::Heading(heading) = node {
        let offset = heading
            .position
            .as_ref()
            .map(|position| position.start.offset as f32)
            .unwrap_or(0.0);
        found.push(HeadingMark {
            level: heading.depth,
            fraction: (offset / len).clamp(0.0, 1.0),
            label: node.to_string(),
        });
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_headings(child, len, found);
        }
    }
}

/// One block's own shape for the minimap — the same shapes `state::document::minimap_rows` draws
/// for an annotated document's host-indexed blocks, computed here straight off the raw source
/// instead. **This is the one minimap** (T-134): the standard viewer's own preview had a second,
/// heading-only strip and the annotation surface a third, denser one drawn one mark per source
/// line; both read as a barcode next to the reference minimap's handful of legible bars. Now both
/// surfaces draw the same shapes, through the same `kit::minimap` primitive and the same
/// `ui::document::mark_style` palette — this walk is the ordinary-tab half, for a buffer with no
/// host block index to read; `ui::document::document_minimap` is the annotated half.
pub struct StructureMark {
    pub kind: crate::state::document::MinimapBlockKind,
    /// The block's own byte offset over the document's total length, `0.0..=1.0` — the same honest
    /// approximation [`HeadingMark::fraction`] uses, for the reason given there.
    pub fraction: f32,
    /// `0.0..=1.0`, this block's own share of a full column width — the widest line it holds
    /// against [`crate::state::document::LINE_LENGTH_CHARS`], mirroring
    /// `state::document::text_rows`'s measure so a file draws the same shape whichever surface
    /// reads it.
    pub length: f32,
}

/// Every top-level block the minimap draws a shape for, in document order. Only the root's own
/// children are read — a fence or a heading nested in a list or a blockquote is prose inside a
/// larger block as far as the minimap is concerned, the same granularity `minimap_rows` reads off
/// the host's own top-level block index.
///
/// Off the shared, cached parse — see [`walks`].
pub fn structure_marks(key: &str, source: &str) -> Rc<Vec<StructureMark>> {
    walks(key, source).1
}

/// What one document's structural walk produced, kept until its source changes.
struct Walked {
    len: usize,
    hash: u64,
    headings: Rc<Vec<HeadingMark>>,
    structure: Rc<Vec<StructureMark>>,
}

thread_local! {
    /// One entry per open document, on [`SCAN_CACHE`]'s own terms and for its own reason.
    static WALK_CACHE: RefCell<HashMap<String, Walked>> = RefCell::new(HashMap::new());
}

/// The navigator's headings and the minimap's block shapes, from **one** parse of the document,
/// kept until the document changes (T-144).
///
/// Both are projections of the same mdast, and both are read from a render function — so each was
/// running `to_mdast` over the whole buffer on every frame, twice per frame together, for a
/// document that had not changed since the last one. That is not a rounding error:
/// `markdown::to_mdast` at `ParseOptions::gfm` costs about 7.7ms on a 50KB document and 38ms on a
/// 150KB one **in release**, so a markdown tab with the minimap on could not reach 60fps on a file
/// of any size no matter what else it did. The fix is the same one [`SCAN_CACHE`] already applies
/// to the fence scan beside it: fingerprint the source, and redo the walk only when it moved.
///
/// **They are still two projections, not one.** They read the tree at different depths on
/// purpose — [`heading_marks`] recurses, so a heading inside a list or a quote is still a heading
/// the navigator lists, while [`structure_marks`] reads only the root's own children, because a
/// block nested inside a larger one is part of that block's shape as far as a minimap is
/// concerned. What they wanted to share was the parse, not the walk.
fn walks(key: &str, source: &str) -> (Rc<Vec<HeadingMark>>, Rc<Vec<StructureMark>>) {
    let (len, hash) = fingerprint(source);
    WALK_CACHE.with_borrow_mut(|cache| {
        let stale =
            !matches!(cache.get(key), Some(cached) if cached.len == len && cached.hash == hash);
        if stale {
            let ast = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).ok();
            let source_len = source.len().max(1) as f32;
            let mut headings = Vec::new();
            let mut structure = Vec::new();
            if let Some(ast) = &ast {
                collect_headings(ast, source_len, &mut headings);
                if let Some(children) = ast.children() {
                    structure.extend(
                        children
                            .iter()
                            .filter_map(|node| structure_mark_of(node, source_len)),
                    );
                }
            }
            cache.insert(
                key.to_string(),
                Walked {
                    len,
                    hash,
                    headings: Rc::new(headings),
                    structure: Rc::new(structure),
                },
            );
        }
        let cached = cache.get(key).expect("just inserted, or already fresh");
        (cached.headings.clone(), cached.structure.clone())
    })
}

fn structure_mark_of(node: &markdown_ast::Node, len: f32) -> Option<StructureMark> {
    use crate::state::document::{LINE_LENGTH_CHARS, MinimapBlockKind, is_image_reference};

    let fraction = node
        .position()
        .map(|position| (position.start.offset as f32 / len).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let widest = |text: &str| -> f32 {
        let widest = text
            .lines()
            .map(|line| line.trim().len())
            .max()
            .unwrap_or(0);
        (widest as f32 / LINE_LENGTH_CHARS).clamp(0.08, 1.0)
    };

    match node {
        markdown_ast::Node::Heading(_) => Some(StructureMark {
            kind: MinimapBlockKind::Heading,
            fraction,
            length: 0.85,
        }),
        markdown_ast::Node::Paragraph(_) => {
            let text = node.to_string();
            if is_image_reference(&text) {
                Some(StructureMark {
                    kind: MinimapBlockKind::Image,
                    fraction,
                    length: 0.55,
                })
            } else {
                Some(StructureMark {
                    kind: MinimapBlockKind::Paragraph,
                    fraction,
                    length: widest(&text),
                })
            }
        }
        markdown_ast::Node::Code(_) => Some(StructureMark {
            kind: MinimapBlockKind::Code,
            fraction,
            length: 1.0,
        }),
        markdown_ast::Node::Table(_) => Some(StructureMark {
            kind: MinimapBlockKind::Table,
            fraction,
            length: widest(&node.to_string()),
        }),
        _ => None,
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

    /// Headings are found in document order, at increasing fractions, each carrying its own
    /// level — the standard viewer minimap's data (T-118).
    #[test]
    fn heading_marks_are_ordered_and_leveled() {
        let source = "# One\n\nbody\n\n## Two\n\nmore body\n\n### Three\n";
        let marks = heading_marks("ordered-and-leveled", source);
        assert_eq!(marks.len(), 3);
        assert_eq!([marks[0].level, marks[1].level, marks[2].level], [1, 2, 3]);
        assert!(marks[0].fraction < marks[1].fraction);
        assert!(marks[1].fraction < marks[2].fraction);
        assert_eq!(marks[0].fraction, 0.0);
    }

    /// No headings, no marks — a document with none draws an empty strip rather than erroring.
    #[test]
    fn heading_marks_of_a_headingless_document_is_empty() {
        assert!(heading_marks("headingless", "just a paragraph, nothing more.\n").is_empty());
    }

    /// The cache is keyed on the source as well as the document: the same key asked twice about
    /// two different documents answers about the second one, not about the first (T-144).
    #[test]
    fn a_changed_document_is_walked_again() {
        let key = "one-key-two-documents";
        assert_eq!(heading_marks(key, "# One\n").len(), 1);
        assert_eq!(heading_marks(key, "# One\n\n## Two\n").len(), 2);
        assert!(heading_marks(key, "no headings here\n").is_empty());
    }

    /// One parse, two projections, at two depths on purpose: a heading inside a list is a heading
    /// the navigator lists and *not* a shape of its own on the minimap, which reads only the
    /// root's own children (T-144).
    #[test]
    fn the_two_walks_read_the_tree_at_different_depths() {
        let source = "# Top\n\n- item\n\n  ## Nested\n";
        assert_eq!(heading_marks("depths", source).len(), 2);
        let structure = structure_marks("depths", source);
        assert_eq!(structure.len(), 1);
        assert_eq!(
            structure[0].kind,
            crate::state::document::MinimapBlockKind::Heading
        );
    }
}
