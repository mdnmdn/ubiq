//! Events to blocks.
//!
//! `pulldown-cmark`'s `into_offset_iter()` is the whole foundation: it hands back every event with
//! the source range it came from, and for a `Start`/`End` pair that range is the *element's* range.
//! So the block table is a recursive descent over a flat event vector, and the ranges are the
//! parser's own rather than anything reconstructed.
//!
//! The events are collected into a vector up front instead of being consumed as a stream. A stream
//! would need an explicit state machine for nesting; an indexable slice needs only a cursor, and
//! the whole file is already in memory anyway.

use std::ops::Range;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag,
};

use crate::{
    Admonition, Block, BlockKind, Cell, ColumnAlign, FrontMatterStyle, Inline, ListItem, Mark,
    content_hash,
};

/// The extensions the viewer's documents actually use.
///
/// Front matter is here because a `---\ntitle: …\n---` header must be its own block: the tree's own
/// documents all carry one, and laying it out as prose is the visible bug.
///
/// Smart punctuation is deliberately *off* — it rewrites text away from the source, and the block
/// table's job is to stay mappable back to bytes.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

type Ev = (Event<'static>, Range<usize>);

struct Cursor<'a> {
    evs: &'a [Ev],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<&Ev> {
        self.evs.get(self.i)
    }

    fn bump(&mut self) {
        self.i += 1;
    }

    /// Consume the `End` that closes the element we are inside, if we are sitting on it.
    fn close(&mut self) {
        if matches!(self.peek(), Some((Event::End(_), _))) {
            self.bump();
        }
    }
}

pub(crate) fn blocks(source: &str) -> Vec<Block> {
    let evs: Vec<Ev> = Parser::new_ext(source, options())
        .into_offset_iter()
        .map(|(e, r)| (e.into_static(), r))
        .collect();
    let mut cur = Cursor { evs: &evs, i: 0 };
    build_blocks(&mut cur, source)
}

fn block(
    ix: usize,
    kind: BlockKind,
    range: Range<usize>,
    src: &str,
    spans: Vec<(Range<usize>, Inline)>,
) -> Block {
    let hash = content_hash(src.get(range.clone()).unwrap_or_default());
    Block {
        ix,
        kind,
        range,
        hash,
        spans,
    }
}

/// Every block at one level of nesting, stopping at the `End` that closes the parent.
fn build_blocks(cur: &mut Cursor, src: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    while let Some((ev, range)) = cur.peek().cloned() {
        match ev {
            Event::End(_) => break,
            Event::Start(tag) => {
                cur.bump();
                let b = build_block(tag, range, cur, src, out.len());
                out.push(b);
            }
            Event::Rule => {
                cur.bump();
                let b = block(out.len(), BlockKind::ThematicBreak, range, src, Vec::new());
                out.push(b);
            }
            _ => {
                // A tight list item carries its inlines with no paragraph wrapper. Give it one, so
                // callers see the same block shape everywhere; `tight` is recorded on the list.
                let spans = collect_inlines(cur, src);
                let Some(first) = spans.first() else {
                    cur.bump(); // Nothing consumed — an event we do not model. Do not spin.
                    continue;
                };
                let r = first.0.start..spans.last().unwrap().0.end;
                let b = block(out.len(), BlockKind::Paragraph, r, src, spans);
                out.push(b);
            }
        }
    }
    out
}

fn build_block(
    tag: Tag<'static>,
    range: Range<usize>,
    cur: &mut Cursor,
    src: &str,
    ix: usize,
) -> Block {
    match tag {
        Tag::Paragraph => {
            let spans = collect_inlines(cur, src);
            cur.close();
            let kind = image_paragraph(&spans).unwrap_or(BlockKind::Paragraph);
            block(ix, kind, range, src, spans)
        }
        Tag::Heading { level, .. } => {
            let spans = collect_inlines(cur, src);
            cur.close();
            let kind = BlockKind::Heading {
                level: level as u8,
                text: plain_text(&spans),
                // Filled by `assign_slugs`: de-duplication needs the whole document.
                slug: String::new(),
            };
            block(ix, kind, range, src, spans)
        }
        Tag::CodeBlock(kind) => {
            // A code block's `Text` events are the code verbatim, fences and indentation already
            // stripped, so concatenating them is byte-identical to what a fence renderer needs.
            let body = collect_literal(cur, |ev| match ev {
                Event::Text(s) => Some(s.to_string()),
                _ => None,
            });
            cur.close();
            let (language, fenced) = match kind {
                CodeBlockKind::Fenced(info) => {
                    // The info string can carry more than a language (`rust,ignore`); the first
                    // word is the language and the rest is the renderer's business.
                    let lang = info.split_whitespace().next().unwrap_or("").to_string();
                    ((!lang.is_empty()).then_some(lang), true)
                }
                CodeBlockKind::Indented => (None, false),
            };
            block(
                ix,
                BlockKind::Code {
                    language,
                    body,
                    fenced,
                },
                range,
                src,
                Vec::new(),
            )
        }
        Tag::HtmlBlock => {
            let body = collect_literal(cur, |ev| match ev {
                Event::Html(s) => Some(s.to_string()),
                _ => None,
            });
            cur.close();
            block(ix, BlockKind::Html { body }, range, src, Vec::new())
        }
        Tag::MetadataBlock(kind) => {
            let body = collect_literal(cur, |ev| match ev {
                Event::Text(s) => Some(s.to_string()),
                _ => None,
            });
            cur.close();
            let style = match kind {
                MetadataBlockKind::YamlStyle => FrontMatterStyle::Yaml,
                MetadataBlockKind::PlusesStyle => FrontMatterStyle::Pluses,
            };
            block(
                ix,
                BlockKind::FrontMatter { style, body },
                range,
                src,
                Vec::new(),
            )
        }
        Tag::BlockQuote(kind) => {
            let blocks = build_blocks(cur, src);
            cur.close();
            let kind = BlockKind::Quote {
                admonition: kind.map(admonition),
                blocks,
            };
            block(ix, kind, range, src, Vec::new())
        }
        Tag::FootnoteDefinition(label) => {
            let blocks = build_blocks(cur, src);
            cur.close();
            let kind = BlockKind::FootnoteDefinition {
                label: label.to_string(),
                blocks,
            };
            block(ix, kind, range, src, Vec::new())
        }
        Tag::List(start) => build_list(start, range, cur, src, ix),
        Tag::Table(alignments) => build_table(alignments, range, cur, src, ix),
        // Inline tags cannot start a block, and the tags this crate does not model (definition
        // lists, which are not enabled) fall through to the same place.
        _ => {
            skip_element(cur);
            block(ix, BlockKind::Other, range, src, Vec::new())
        }
    }
}

fn build_list(
    start: Option<u64>,
    range: Range<usize>,
    cur: &mut Cursor,
    src: &str,
    ix: usize,
) -> Block {
    let mut items = Vec::new();
    // CommonMark's own definition of a loose list is the one the parser already applied: it wraps
    // item content in paragraphs exactly when the list is loose. No second guess needed.
    let mut loose = false;

    while let Some((ev, item_range)) = cur.peek().cloned() {
        match ev {
            Event::Start(Tag::Item) => {
                cur.bump();
                let mut checked = None;
                // `- [x] …` puts the marker either directly in the item or inside its paragraph,
                // depending on tightness. Take it here when it is loose in the item.
                if let Some((Event::TaskListMarker(b), _)) = cur.peek().cloned() {
                    cur.bump();
                    checked = Some(b);
                }
                if matches!(cur.peek(), Some((Event::Start(Tag::Paragraph), _))) {
                    loose = true;
                }
                let blocks = build_blocks(cur, src);
                cur.close();
                if checked.is_none() {
                    checked = task_marker(&blocks);
                }
                items.push(ListItem {
                    range: item_range,
                    checked,
                    blocks,
                });
            }
            Event::End(_) => break,
            _ => cur.bump(),
        }
    }
    cur.close();

    let kind = BlockKind::List {
        ordered: start.is_some(),
        start,
        tight: !loose,
        items,
    };
    block(ix, kind, range, src, Vec::new())
}

fn build_table(
    alignments: Vec<Alignment>,
    range: Range<usize>,
    cur: &mut Cursor,
    src: &str,
    ix: usize,
) -> Block {
    let mut head = Vec::new();
    let mut rows = Vec::new();

    while let Some((ev, _)) = cur.peek().cloned() {
        match ev {
            Event::Start(Tag::TableHead) => {
                cur.bump();
                head = build_cells(cur, src);
            }
            Event::Start(Tag::TableRow) => {
                cur.bump();
                rows.push(build_cells(cur, src));
            }
            Event::End(_) => break,
            _ => cur.bump(),
        }
    }
    cur.close();

    let kind = BlockKind::Table {
        alignments: alignments.into_iter().map(column_align).collect(),
        head,
        rows,
    };
    block(ix, kind, range, src, Vec::new())
}

/// The cells of one row, consuming the `End` of the row itself.
fn build_cells(cur: &mut Cursor, src: &str) -> Vec<Cell> {
    let mut out = Vec::new();
    while let Some((ev, cell_range)) = cur.peek().cloned() {
        match ev {
            Event::Start(Tag::TableCell) => {
                cur.bump();
                let spans = collect_inlines(cur, src);
                cur.close();
                out.push(Cell {
                    range: cell_range,
                    spans,
                });
            }
            Event::End(_) => break,
            _ => cur.bump(),
        }
    }
    cur.close();
    out
}

/// The flat inline stream of the current block, stopping at the `End` that closes it.
///
/// Nesting is balanced by construction, so a mark stack is enough to give `Exit` the same `Mark`
/// its `Enter` carried — a renderer never has to pair them itself.
fn collect_inlines(cur: &mut Cursor, _src: &str) -> Vec<(Range<usize>, Inline)> {
    let mut out: Vec<(Range<usize>, Inline)> = Vec::new();
    let mut stack: Vec<Mark> = Vec::new();

    while let Some((ev, range)) = cur.peek().cloned() {
        match &ev {
            // Our own closing tag — leave it for the caller.
            Event::End(_) if stack.is_empty() => break,
            // A block started (a nested list under a tight item); that is not inline content.
            Event::Start(t) if mark_of(t).is_none() => break,
            Event::Rule => break,
            _ => {}
        }
        cur.bump();
        match ev {
            Event::Start(t) => {
                let mark = mark_of(&t).expect("checked above");
                stack.push(mark.clone());
                out.push((range, Inline::Enter(mark)));
            }
            Event::End(_) => {
                let mark = stack.pop().expect("balanced by construction");
                out.push((range, Inline::Exit(mark)));
            }
            Event::Text(s) => out.push((range, Inline::Text(s.to_string()))),
            Event::Code(s) => out.push((range, Inline::Code(s.to_string()))),
            Event::InlineHtml(s) | Event::Html(s) => out.push((range, Inline::Html(s.to_string()))),
            Event::FootnoteReference(s) => {
                out.push((range, Inline::FootnoteReference(s.to_string())))
            }
            Event::TaskListMarker(b) => out.push((range, Inline::TaskMarker(b))),
            Event::SoftBreak => out.push((range, Inline::SoftBreak)),
            Event::HardBreak => out.push((range, Inline::HardBreak)),
            // Math is not enabled; if it ever is, its source text is the honest fallback.
            Event::InlineMath(s) | Event::DisplayMath(s) => {
                out.push((range, Inline::Text(s.to_string())))
            }
            Event::Rule => break,
        }
    }
    out
}

/// Concatenate the literal payload of a leaf block — code and HTML bodies.
fn collect_literal(cur: &mut Cursor, pick: fn(&Event<'static>) -> Option<String>) -> String {
    let mut out = String::new();
    while let Some((ev, _)) = cur.peek().cloned() {
        if matches!(ev, Event::End(_)) {
            break;
        }
        cur.bump();
        if let Some(s) = pick(&ev) {
            out.push_str(&s);
        }
    }
    out
}

/// Consume a balanced element whose `Start` was already taken.
fn skip_element(cur: &mut Cursor) {
    let mut depth = 0usize;
    while let Some((ev, _)) = cur.peek().cloned() {
        match ev {
            Event::Start(_) => {
                depth += 1;
                cur.bump();
            }
            Event::End(_) => {
                cur.bump();
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => cur.bump(),
        }
    }
}

fn mark_of(tag: &Tag<'_>) -> Option<Mark> {
    match tag {
        Tag::Emphasis => Some(Mark::Emphasis),
        Tag::Strong => Some(Mark::Strong),
        Tag::Strikethrough => Some(Mark::Strikethrough),
        Tag::Link {
            dest_url, title, ..
        } => Some(Mark::Link {
            dest: dest_url.to_string(),
            title: title.to_string(),
        }),
        Tag::Image {
            dest_url, title, ..
        } => Some(Mark::Image {
            dest: dest_url.to_string(),
            title: title.to_string(),
        }),
        _ => None,
    }
}

/// The readable text of an inline stream — what a heading's slug and an outline entry are made of.
fn plain_text(spans: &[(Range<usize>, Inline)]) -> String {
    let mut out = String::new();
    for (_, inline) in spans {
        match inline {
            Inline::Text(s) | Inline::Code(s) => out.push_str(s),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// A paragraph that is nothing but one image is a figure, not prose — §13.2 gives it a row rule of
/// its own, so the kind has to say so.
fn image_paragraph(spans: &[(Range<usize>, Inline)]) -> Option<BlockKind> {
    let (_, Inline::Enter(Mark::Image { dest, title })) = spans.first()? else {
        return None;
    };
    if !matches!(spans.last()?, (_, Inline::Exit(Mark::Image { .. }))) {
        return None;
    }
    Some(BlockKind::Image {
        dest: dest.clone(),
        title: title.clone(),
        alt: plain_text(&spans[1..spans.len() - 1]),
    })
}

/// A tight list item puts its task marker inside the synthetic paragraph, so look there too.
fn task_marker(blocks: &[Block]) -> Option<bool> {
    blocks.first()?.spans.iter().find_map(|(_, i)| match i {
        Inline::TaskMarker(b) => Some(*b),
        _ => None,
    })
}

fn admonition(kind: BlockQuoteKind) -> Admonition {
    match kind {
        BlockQuoteKind::Note => Admonition::Note,
        BlockQuoteKind::Tip => Admonition::Tip,
        BlockQuoteKind::Important => Admonition::Important,
        BlockQuoteKind::Warning => Admonition::Warning,
        BlockQuoteKind::Caution => Admonition::Caution,
    }
}

fn column_align(a: Alignment) -> ColumnAlign {
    match a {
        Alignment::None => ColumnAlign::Default,
        Alignment::Left => ColumnAlign::Left,
        Alignment::Center => ColumnAlign::Center,
        Alignment::Right => ColumnAlign::Right,
    }
}
