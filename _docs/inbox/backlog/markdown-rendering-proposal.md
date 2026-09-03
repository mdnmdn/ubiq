---
id: inbox-markdown
title: Proposal — Markdown rendering, UI typography and frontmatter
kind: proposal
status: proposal
summary: Render Markdown at the project UI font size rather than an unscaled default, synchronize it with zoom, and collapse YAML frontmatter into a monospaced disclosure block by default.
read_when: you are deciding how Markdown documents are rendered, how their typography scales with UI zoom, or how document frontmatter is displayed
updated: 2026-09-03
depends_on: [inbox-viewers, tech-ui, feat-workbench]
---

# Proposal — Markdown rendering, UI typography and frontmatter

Ubiq renders Markdown files into a live preview using the component library's `TextView`.
Today, that preview ignores the application's font size, ignores project zoom, and leaves
YAML frontmatter to render as plain paragraphs sandwiched between two horizontal rules.

This proposes two related changes: **rendering Markdown at the same font size as the UI**,
synchronized with project zoom, and **separating YAML frontmatter into a monospaced,
collapsible disclosure block that is closed by default**.

## 1. Where it stands

**Markdown text ignores UI font sizing.**
`crates/ubiq/src/ui/viewer/markdown.rs` builds a `TextView::markdown` element without setting
a text size. The element falls back to `gpui-component`'s default typography (16px base size).
Meanwhile, Ubiq's UI chrome is laid out at 12.5px–13px (`crates/ubiq/src/theme.rs`,
`EDITOR_FONT_SIZE`), and the file buffer editor renders at `app.ui_font_size(cx)`.

**Split layout produces a typographic mismatch.**
In `ViewLayout::Split`, the left half (the editor buffer) draws at the project's font size
(default 13px), while the right half (the Markdown preview) draws at 16px. Stepping font size
with `⌘+` and `⌘-` (`AppState::nudge_ui_font_size`) scales the editor buffer, explorer tree,
and terminal panes, but the Markdown preview stays frozen at the component library's default.

**Other Markdown call sites share the defect.**
Both `crates/ubiq/src/ui/chat/transcript.rs` (assistant message markdown blocks) and
`crates/ubiq/src/ui/board/form.rs` (task description previews) construct `TextView::markdown`
without specifying `.text_size()`.

**Frontmatter renders as corrupted content.**
Ubiq documents and agent specification files begin with YAML frontmatter enclosed by `---`.
Because the Markdown viewer hands the raw document string straight to `TextView`, the CommonMark
parser treats the leading and trailing `---` delimiters as horizontal rules (`<hr>`). The
metadata fields (`id: ...`, `title: ...`, `summary: ...`) render as ordinary body paragraphs in
proportional text. The metadata is permanently visible, takes up vertical space before the
actual title, and lacks monospace formatting or visual separation.

## 2. The rules

1. **Markdown body text matches the UI font size.** The base font size for Markdown elements
   derives from the project's font size setting (`AppState::ui_font_size_or_default`).
2. **Zoom is symmetric across split panes.** In `ViewLayout::Split`, the source buffer and the
   preview share the exact same base font size and scale together under zoom.
3. **Frontmatter is metadata, not body text.** Leading YAML frontmatter is extracted before
   passing source to the Markdown parser. The parser never sees the opening or closing `---`.
4. **Frontmatter is monospaced and collapsed by default.** An interactive disclosure bar sits
   at the head of the preview. When closed, it reports a clean summary; when opened, it reveals
   the raw YAML in `MONO_FONT`.
5. **The preview scrolls as one document.** The frontmatter bar and the Markdown body share a
   single scroll container, so scrolling down reveals content without pinning an awkward header.

## 3. Font scaling and UI synchronization

`AppState` tracks the project's zoomable font preference through `ui_font_size(cx)` and
`ui_font_size_or_default(cx)` in `crates/ubiq/src/app.rs`.

`crates/ubiq/src/ui/viewer/mod.rs` already resolves this size when rendering the editor buffer:

```rust
let font_size = app.ui_font_size(cx);
match doc.viewer() {
    ViewerKind::Markdown => viewer::markdown::render(app, &key, &source, font_size),
    // ...
}
```

In `crates/ubiq/src/ui/viewer/markdown.rs`, `render()` takes `font_size: Option<f32>` and applies
it directly to the `TextView`:

```rust
let size = font_size.unwrap_or(theme::EDITOR_FONT_SIZE);
TextView::markdown(eid("md", key), body.to_string())
    .text_size(px(size))
    .markdown_extensions(extensions().clone())
    .selectable(true)
```

Because `gpui-component`'s text layout scales headings (`H1` through `H6`), blockquotes, and
code blocks relative to the element's configured `text_size`, setting the base size scales the
entire document proportionally.

The chat transcript (`crates/ubiq/src/ui/chat/transcript.rs`) and tasks board form
(`crates/ubiq/src/ui/board/form.rs`) apply `theme::EDITOR_FONT_SIZE` (13px) to their respective
`TextView::markdown` instances, eliminating oversized body copy across all surfaces.

## 4. Frontmatter extraction

Frontmatter extraction happens at render time as a pure function over the source string in
`crates/ubiq/src/ui/viewer/markdown.rs`:

```rust
pub fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
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
```

Properties of this split:
- **Zero allocation for content without frontmatter:** Documents without a leading `---`
  return `(None, source)` immediately.
- **Tolerant of line endings:** Handles both LF and CRLF delimiters.
- **Clean body output:** The body passed to `TextView::markdown` starts after the closing `---`,
  preventing spurious horizontal rules or misparsed key-value text.

## 5. Monospaced disclosure component

When frontmatter is detected, the viewer renders a collapsible block at the top of the preview
using `crates/ubiq/src/ui/kit/controls.rs`, `disclosure()`:

```rust
fn frontmatter_bar(
    key: &str,
    raw_yaml: &str,
    open: bool,
    font_size: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let summary = mono(frontmatter_summary(raw_yaml), theme::text_faint())
        .text_size(px(font_size - 1.5));

    div()
        .flex()
        .flex_col()
        .child(disclosure(
            eid2("md-fm", key, "disc"),
            "Frontmatter",
            summary,
            open,
            cx.listener(move |this, _, _, cx| this.toggle_frontmatter(key, cx)),
        ))
        .children(open.then(|| {
            div()
                .p_3()
                .bg(theme::pane_bg())
                .border_b_1()
                .border_color(theme::border())
                .child(
                    mono(raw_yaml.to_string(), theme::text_muted())
                        .text_size(px(font_size - 1.0)),
                )
        }))
        .into_any_element()
}
```

### Presentation characteristics
- **Collapsed state (default):** Shows a compact 32px bar with a chevron, a `"Frontmatter"`
  label, and a faint inline summary (e.g. `id: ... | title: ...` or line count).
- **Expanded state:** Reveals the full YAML content inside a bordered container with
  `theme::pane_bg()`, `theme::MONO_FONT`, and `theme::text_muted()`.
- **Monospaced formatting:** Kept in `MONO_FONT` at `font_size - 1.0` (12px default), preserving
  indentation, arrays, and syntax without wrapping prematurely.

## 6. State and tab lifecycle

Whether frontmatter is open or closed is per-tab UI state:

1. **Model field on `OpenFile`:**
   `crates/ubiq/src/state/editor.rs` adds `pub frontmatter_open: bool` to `OpenFile`.
   Constructors `OpenFile::opening` and `OpenFile::temporary` initialize this field to `false`.
2. **Mutation action on `AppState`:**
   `crates/ubiq/src/app.rs` adds `toggle_frontmatter(&mut self, key: &str, cx: &mut Context<Self>)`,
   which flips the flag on the targeted tab and notifies context for redraw.
3. **Restoration:**
   Because frontmatter is designed to be tucked away while reading documents, defaulting to `false`
   ensures newly opened documents start clean without requiring persistence across restarts.

## 7. Container layout and scrolling

Currently, `crates/ubiq/src/ui/viewer/markdown.rs` configures `.scrollable(true)` directly on the
`TextView`. However, placing an interactive element outside a scrollable `TextView` causes the
header to stay permanently pinned at the top while only the body scrolls beneath it.

To create a natural document reading experience:
- The parent container (`crates/ubiq/src/ui/viewer/markdown.rs`) owns the scroll view:
  `div().flex().flex_col().flex_1().overflow_scroll()`.
- `TextView` is set to `.scrollable(false)`, letting it measure and lay out its full height.
- The frontmatter bar sits at the head of this container and scrolls with the document.

## 8. What this costs

- **Header measurement:** Splitting frontmatter scans the head of the file for the closing
  `---`. Since frontmatter typically sits within the first 50 lines, this scan completes in
  sub-microsecond time and does not touch the parser thread.
- **Two elements in the viewer:** Instead of a single `TextView`, files with frontmatter render
  a disclosure container followed by `TextView`. In the common case without frontmatter, the
  disclosure element is omitted entirely.
- **No parser fork required:** Neither `gpui-component` nor `markdown` needs to be patched;
  frontmatter handling remains cleanly encapsulated within Ubiq's viewer layer.

## Related docs

- [`file-viewers-proposal.md`](./file-viewers-proposal.md) — the file viewer architecture and layout modes
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — typography constants, theme tokens, and kit controls
- [`../features/workbench.md`](../features/workbench.md) — the editor panel and dock layout
