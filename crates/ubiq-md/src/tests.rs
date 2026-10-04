//! The block table's invariant, tested from both directions.
//!
//! The load-bearing test is `ranges_round_trip`: if `&source[block.range]` does not re-parse to the
//! same kind, the range is wrong, and every consumer of the table — minimap, scroll sync, anchors,
//! block-to-editor swap — is wrong with it.

use super::*;

/// Kinds compared by name, because the payloads differ between a nested parse and a root one.
fn kind_name(k: &BlockKind) -> &'static str {
    match k {
        BlockKind::FrontMatter { .. } => "frontmatter",
        BlockKind::Heading { .. } => "heading",
        BlockKind::Paragraph => "paragraph",
        BlockKind::Image { .. } => "image",
        BlockKind::List { .. } => "list",
        BlockKind::Code { .. } => "code",
        BlockKind::Quote { .. } => "quote",
        BlockKind::Table { .. } => "table",
        BlockKind::ThematicBreak => "thematic-break",
        BlockKind::Html { .. } => "html",
        BlockKind::FootnoteDefinition { .. } => "footnote",
        BlockKind::Other => "other",
    }
}

const SAMPLE: &str = "\
---
title: Sample
tags: [a, b]
---

# The title

A paragraph with *emphasis*, **strong**, `code`, a [link](https://example.com \"T\") and a
soft break.

## Notes

- one
- two with **bold**
- [ ] a task
- [x] a done task

1. first
2. second

> A quote.
>
> With two paragraphs.

```rust
fn main() {
    println!(\"hi\");
}
```

| Left | Centre | Right |
|:---|:---:|---:|
| a | b | c |
| d | `e` | f |

---

## Notes

Text with a footnote[^1] and ~~strikethrough~~.

![a picture](pic.png)

[^1]: The footnote body.
";

fn doc() -> Document {
    parse(SAMPLE)
}

#[test]
fn blocks_are_ordered_and_disjoint() {
    let d = doc();
    assert!(!d.blocks.is_empty());
    for (i, b) in d.blocks.iter().enumerate() {
        assert_eq!(b.ix, i, "ix is the row index");
        assert!(b.range.start < b.range.end, "block {i} is empty");
        assert!(b.range.end <= d.source.len());
        if i > 0 {
            assert!(
                d.blocks[i - 1].range.end <= b.range.start,
                "block {i} overlaps its predecessor"
            );
        }
    }
}

#[test]
fn ranges_round_trip() {
    let d = doc();
    for b in &d.blocks {
        let slice = &d.source[b.range.clone()];
        let re = parse(slice);
        assert_eq!(
            re.blocks.len(),
            1,
            "block {} ({}) re-parsed to {} blocks: {slice:?}",
            b.ix,
            kind_name(&b.kind),
            re.blocks.len()
        );
        assert_eq!(
            kind_name(&re.blocks[0].kind),
            kind_name(&b.kind),
            "block {} changed kind when re-parsed alone: {slice:?}",
            b.ix
        );
    }
}

#[test]
fn offsets_map_back() {
    let d = doc();
    for b in &d.blocks {
        let mid = b.range.start + (b.range.len() / 2);
        assert_eq!(d.block_at_offset(b.range.start), Some(b.ix));
        assert_eq!(d.block_at_offset(mid), Some(b.ix));
        let (ix, local) = d.offset_to_block_and_local_offset(mid).unwrap();
        assert_eq!(ix, b.ix);
        assert_eq!(b.range.start + local, mid);
        assert_eq!(d.range_of_block(b.ix), Some(b.range.clone()));
    }
    // The blank line after a block belongs to the block it follows.
    let first_end = d.blocks[0].range.end;
    assert_eq!(d.block_at_offset(first_end), None);
    assert_eq!(d.block_at_or_before_offset(first_end), Some(0));
    assert_eq!(
        d.block_at_or_before_offset(usize::MAX),
        Some(d.blocks.len() - 1)
    );
}

#[test]
fn slugs_deduplicate() {
    let d = doc();
    let slugs: Vec<_> = d.headings().into_iter().map(|h| h.slug).collect();
    assert_eq!(slugs, vec!["the-title", "notes", "notes-1"]);
    assert_eq!(d.block_for_slug("notes-1"), Some(d.headings()[2].block_ix));
    assert_eq!(d.block_for_slug("nope"), None);

    // GitHub's rule: punctuation dropped, spaces hyphenated, case folded.
    assert_eq!(slugify("Hello, World! — §3"), "hello-world--3");
    assert_eq!(slugify("`code` and stuff"), "code-and-stuff");
}

#[test]
fn outline_agrees_with_headings() {
    let d = doc();
    let outline = d.outline();
    let headings = d.headings();
    assert_eq!(outline.len(), headings.len());
    for (o, h) in outline.iter().zip(&headings) {
        assert_eq!(
            (o.0, o.1, o.2.as_str()),
            (h.block_ix, h.level, h.text.as_str())
        );
    }
    assert_eq!(outline[0], (1, 1, "The title".to_string()));
}

#[test]
fn front_matter_is_not_prose() {
    let d = doc();
    match &d.blocks[0].kind {
        BlockKind::FrontMatter { style, body } => {
            assert_eq!(*style, FrontMatterStyle::Yaml);
            assert!(body.contains("title: Sample"));
        }
        other => panic!("front matter parsed as {}", kind_name(other)),
    }
    // And a `---` that is *not* at the top stays a thematic break.
    let d2 = parse("para\n\n---\n\nmore\n");
    assert!(matches!(d2.blocks[1].kind, BlockKind::ThematicBreak));
}

#[test]
fn fenced_code_survives_intact() {
    let d = doc();
    let code = d
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Code {
                language,
                body,
                fenced,
            } => Some((language.clone(), body.clone(), *fenced)),
            _ => None,
        })
        .expect("a fenced block");
    assert_eq!(code.0.as_deref(), Some("rust"));
    assert!(code.2);
    assert_eq!(code.1, "fn main() {\n    println!(\"hi\");\n}\n");
}

#[test]
fn table_keeps_its_shape() {
    let d = doc();
    let (alignments, head, rows) = d
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Table {
                alignments,
                head,
                rows,
            } => Some((alignments.clone(), head.clone(), rows.clone())),
            _ => None,
        })
        .expect("a table");
    assert_eq!(
        alignments,
        vec![ColumnAlign::Left, ColumnAlign::Center, ColumnAlign::Right]
    );
    assert_eq!(head.len(), 3);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].len(), 3);
    // Cells carry inlines, not re-parseable text.
    assert!(
        rows[1][1]
            .spans
            .iter()
            .any(|(_, i)| *i == Inline::Code("e".into()))
    );
    // And every cell range points at its own source.
    assert_eq!(&d.source[head[0].range.clone()], " Left ");
}

#[test]
fn lists_carry_items_tightness_and_tasks() {
    let d = doc();
    let lists: Vec<_> = d
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::List {
                ordered,
                start,
                tight,
                items,
            } => Some((*ordered, *start, *tight, items.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(lists.len(), 2);

    let (ordered, start, tight, items) = &lists[0];
    assert!(!ordered && start.is_none() && *tight);
    assert_eq!(items.len(), 4);
    assert_eq!(items[2].checked, Some(false));
    assert_eq!(items[3].checked, Some(true));
    assert_eq!(items[0].blocks.len(), 1);
    assert!(&d.source[items[0].range.clone()].starts_with("- one"));

    let (ordered, start, ..) = &lists[1];
    assert!(*ordered);
    assert_eq!(*start, Some(1));
}

#[test]
fn inline_spans_are_balanced_and_map_to_source() {
    let d = doc();
    let para = d
        .blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Paragraph))
        .unwrap();

    let mut depth = 0i32;
    for (range, inline) in &para.spans {
        assert!(para.range.start <= range.start && range.end <= para.range.end);
        match inline {
            Inline::Enter(_) => depth += 1,
            Inline::Exit(_) => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0);
    }
    assert_eq!(depth, 0, "enter/exit are balanced");

    let link = para
        .spans
        .iter()
        .find_map(|(r, i)| match i {
            Inline::Enter(Mark::Link { dest, title }) => {
                Some((r.clone(), dest.clone(), title.clone()))
            }
            _ => None,
        })
        .expect("the link");
    assert_eq!(link.1, "https://example.com");
    assert_eq!(link.2, "T");
    // An `Enter` carries the whole element's range, so a renderer never pairs events itself.
    assert!(d.source[link.0].starts_with("[link]("));

    assert!(para.spans.iter().any(|(_, i)| *i == Inline::SoftBreak));
    assert!(
        para.spans
            .iter()
            .any(|(_, i)| *i == Inline::Code("code".into()))
    );
}

#[test]
fn image_only_paragraph_is_its_own_kind() {
    let d = doc();
    let img = d
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Image { dest, alt, .. } => Some((dest.clone(), alt.clone())),
            _ => None,
        })
        .expect("an image block");
    assert_eq!(img, ("pic.png".to_string(), "a picture".to_string()));
}

#[test]
fn quotes_footnotes_and_strikethrough_are_modelled() {
    let d = doc();
    let quote = d
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Quote { blocks, .. } => Some(blocks.clone()),
            _ => None,
        })
        .expect("a quote");
    assert_eq!(quote.len(), 2, "a quote holds its own block table");

    assert!(
        d.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::FootnoteDefinition { .. }))
    );

    let has_ref = d.blocks.iter().any(|b| {
        b.spans
            .iter()
            .any(|(_, i)| matches!(i, Inline::FootnoteReference(_)))
    });
    assert!(has_ref, "the footnote reference survived as an inline");

    let has_strike = d.blocks.iter().any(|b| {
        b.spans
            .iter()
            .any(|(_, i)| *i == Inline::Enter(Mark::Strikethrough))
    });
    assert!(has_strike);
}

#[test]
fn admonition_quotes_are_recognised() {
    let d = parse("> [!WARNING]\n> Careful.\n");
    match &d.blocks[0].kind {
        BlockKind::Quote { admonition, .. } => assert_eq!(*admonition, Some(Admonition::Warning)),
        other => panic!("got {}", kind_name(other)),
    }
}

#[test]
fn content_hash_is_stable_and_discriminating() {
    assert_eq!(content_hash("abc"), content_hash("abc"));
    assert_ne!(content_hash("abc"), content_hash("abd"));
    let d = doc();
    let again = parse(SAMPLE);
    for (a, b) in d.blocks.iter().zip(&again.blocks) {
        assert_eq!(a.hash, b.hash);
    }
}

#[test]
fn reparse_of_a_middle_edit_is_local() {
    let mut d = doc();
    let before = d.blocks.len();
    let edited = SAMPLE.replace("A quote.", "A quote, edited.");
    let delta = d.reparse(&edited);

    assert_eq!(d.blocks.len(), before, "no block appeared or vanished");
    assert_eq!(delta.old_len, 1, "exactly one block changed");
    assert_eq!(delta.new_len, 1);
    assert!(delta.first_changed > 0, "the prefix was recognised");
    assert!(
        delta.first_changed + delta.old_len < before,
        "the suffix was recognised too — the delta must not span the document"
    );
    assert!(matches!(
        d.blocks[delta.first_changed].kind,
        BlockKind::Quote { .. }
    ));
}

#[test]
fn reparse_reports_insertions_deletions_and_no_ops() {
    let mut d = parse("a\n\nb\n\nc\n");
    assert!(d.reparse("a\n\nb\n\nc\n").is_empty());

    let mut d2 = parse("a\n\nb\n\nc\n");
    let delta = d2.reparse("a\n\nNEW\n\nb\n\nc\n");
    assert_eq!(
        delta,
        BlockDelta {
            first_changed: 1,
            old_len: 0,
            new_len: 1
        }
    );

    let delta = d.reparse("a\n\nc\n");
    assert_eq!(
        delta,
        BlockDelta {
            first_changed: 1,
            old_len: 1,
            new_len: 0
        }
    );
}

#[test]
fn empty_and_degenerate_sources() {
    assert!(parse("").blocks.is_empty());
    assert!(parse("\n\n   \n").blocks.is_empty());
    assert_eq!(parse("").block_at_offset(0), None);
    assert_eq!(Document::default().outline(), vec![]);
}

// --- GFM bare autolinks (roadmap item 11) -------------------------------------------------------

/// Every link in a span list: `(range, destination, visible text)`.
fn links(spans: &[(Range<usize>, Inline)]) -> Vec<(Range<usize>, String, String)> {
    let mut out = Vec::new();
    let mut open: Option<(Range<usize>, String, String)> = None;
    for (r, inline) in spans {
        match inline {
            Inline::Enter(Mark::Link { dest, .. }) => {
                open = Some((r.clone(), dest.clone(), String::new()))
            }
            Inline::Exit(Mark::Link { .. }) => {
                if let Some(l) = open.take() {
                    out.push(l);
                }
            }
            Inline::Text(t) => {
                if let Some(l) = open.as_mut() {
                    l.2.push_str(t);
                }
            }
            _ => {}
        }
    }
    out
}

/// The links of the first block — the paragraph case, which is most of these tests.
fn first_links(source: &str) -> Vec<(Range<usize>, String, String)> {
    let d = parse(source);
    links(&d.blocks[0].spans)
}

/// Just the destinations, from the whole tree including every container.
fn all_dests(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks {
        out.extend(links(&b.spans).into_iter().map(|l| l.1));
        match &b.kind {
            BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                out.extend(all_dests(blocks))
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    out.extend(all_dests(&item.blocks));
                }
            }
            BlockKind::Table { head, rows, .. } => {
                for cell in head.iter().chain(rows.iter().flatten()) {
                    out.extend(links(&cell.spans).into_iter().map(|l| l.1));
                }
            }
            _ => {}
        }
    }
    out
}

/// `find_autolinks` reduced to `(matched text, destination)` — the rule-level assertion.
fn found(text: &str) -> Vec<(&str, String)> {
    find_autolinks(text)
        .into_iter()
        .map(|(r, dest)| (&text[r], dest))
        .collect()
}

#[test]
fn autolink_links_a_plain_url_in_a_sentence() {
    let l = first_links("See https://example.com/a for more.\n");
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].1, "https://example.com/a");
    assert_eq!(l[0].2, "https://example.com/a");

    // The text either side survives as text, unsplit and in order.
    let d = parse("See https://example.com/a for more.\n");
    let text: String = d.blocks[0]
        .spans
        .iter()
        .filter_map(|(_, i)| match i {
            Inline::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "See https://example.com/a for more.");
}

#[test]
fn autolink_excludes_a_closing_paren_it_does_not_own() {
    let l = first_links("(see https://x.com/a)\n");
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].1, "https://x.com/a");
    assert_eq!(l[0].2, "https://x.com/a");
}

#[test]
fn autolink_keeps_balanced_parens() {
    assert_eq!(
        found("https://x.com/a_(b)"),
        vec![("https://x.com/a_(b)", "https://x.com/a_(b)".to_string())]
    );
    let l = first_links("https://en.wikipedia.org/wiki/Rust_(programming_language)\n");
    assert_eq!(
        l[0].1,
        "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
    // One unbalanced `)` still goes; a balanced pair inside it does not.
    assert_eq!(found("(https://x.com/a_(b))")[0].0, "https://x.com/a_(b)");
}

#[test]
fn autolink_strips_trailing_punctuation() {
    for (text, want) in [
        ("https://a.com/x.", "https://a.com/x"),
        ("https://a.com/x!", "https://a.com/x"),
        ("https://a.com/x?", "https://a.com/x"),
        ("https://a.com/x,", "https://a.com/x"),
        ("https://a.com/x:", "https://a.com/x"),
        ("https://a.com/x*", "https://a.com/x"),
        ("https://a.com/x_", "https://a.com/x"),
        ("https://a.com/x~", "https://a.com/x"),
        ("https://a.com/x'", "https://a.com/x"),
        ("https://a.com/x\"", "https://a.com/x"),
        ("https://a.com/x?!.,", "https://a.com/x"),
        // A trailing dot on the bare host too — the domain must still validate after stripping.
        ("https://a.com.", "https://a.com"),
    ] {
        let f = found(text);
        assert_eq!(f.len(), 1, "{text:?}");
        assert_eq!(f[0].0, want, "{text:?}");
    }
    // A query string's `?` is inside the match, not trailing.
    assert_eq!(found("https://a.com/x?q=1")[0].0, "https://a.com/x?q=1");
}

#[test]
fn autolink_semicolon_of_an_entity_is_excluded() {
    assert_eq!(found("https://a.com/?x=1&amp;")[0].0, "https://a.com/?x=1");
    // A bare `;` is not an entity, so it stays part of the URL.
    assert_eq!(found("https://a.com/?x=1;")[0].0, "https://a.com/?x=1;");
}

#[test]
fn autolink_www_gets_an_http_scheme() {
    let l = first_links("Visit www.example.com/docs now.\n");
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].1, "http://www.example.com/docs");
    assert_eq!(
        l[0].2, "www.example.com/docs",
        "the visible text keeps www."
    );
}

#[test]
fn autolink_email_becomes_mailto() {
    let l = first_links("Mail foo.bar+x@example.com please.\n");
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].1, "mailto:foo.bar+x@example.com");
    assert_eq!(l[0].2, "foo.bar+x@example.com");

    // GFM's domain rules: two segments at least, no trailing hyphen, no underscore in the last two.
    assert!(found("foo@localhost").is_empty());
    assert!(found("foo@bar-").is_empty());
    assert!(found("foo@bar_baz.com").is_empty());
    assert_eq!(found("foo@bar.com.")[0].0, "foo@bar.com");
}

#[test]
fn autolink_needs_a_word_boundary() {
    assert!(found("xxhttps://a.com").is_empty());
    assert!(found("mailto:foo@bar.com").is_empty(), "not at a boundary");
    // Whitespace and GFM's `*_~(` are boundaries; the delimiter itself is not in the match.
    assert_eq!(found("*https://a.com*")[0].0, "https://a.com");
    assert_eq!(found("~https://a.com")[0].0, "https://a.com");
    // A bare `www` or a schemeless host is not a link.
    assert!(found("www.").is_empty());
    assert!(found("example.com").is_empty());
}

#[test]
fn autolink_leaves_existing_links_alone() {
    // A markdown link whose visible text is a bare URL: one link, the authored destination.
    let l = first_links("[https://example.com](https://other.com)\n");
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].1, "https://other.com");
    assert_eq!(l[0].2, "https://example.com");

    // An angle autolink is already a link and must not be split either.
    let l = first_links("<https://example.com>\n");
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].1, "https://example.com");

    // Nor an image's alt text, which cannot hold a link.
    let d = parse("![see https://example.com](pic.png)\n");
    assert!(all_dests(&d.blocks).is_empty());
}

#[test]
fn autolink_leaves_code_and_html_alone() {
    let d = parse("Use `https://example.com` or not.\n");
    assert!(links(&d.blocks[0].spans).is_empty());
    assert!(
        d.blocks[0]
            .spans
            .iter()
            .any(|(_, i)| *i == Inline::Code("https://example.com".into()))
    );

    // Inline HTML is literal, and a text span that was unescaped (its length no longer matches its
    // range) is skipped rather than mapped to the wrong bytes.
    let mut spans = vec![
        (0..30, Inline::Html("<a href=\"http://x.com\">".into())),
        (30..40, Inline::Text("http://x.com".into())),
    ];
    let before = spans.clone();
    linkify_spans(&mut spans);
    assert_eq!(spans, before);
}

#[test]
fn autolink_ranges_stay_exact() {
    let source = "Go to https://example.com/a?b=1, or www.x.io, or mail a@b.com.\n";
    let d = parse(source);
    let l = links(&d.blocks[0].spans);
    assert_eq!(l.len(), 3);
    // Every link's range is the URL's own bytes in the source, with no punctuation either side.
    assert_eq!(&d.source[l[0].0.clone()], "https://example.com/a?b=1");
    assert_eq!(&d.source[l[1].0.clone()], "www.x.io");
    assert_eq!(&d.source[l[2].0.clone()], "a@b.com");
    for (range, _, text) in &l {
        assert_eq!(&d.source[range.clone()], text.as_str());
    }

    // And the split left the paragraph's spans contiguous and inside the block.
    let para = &d.blocks[0];
    let mut prev = para.range.start;
    for (range, inline) in &para.spans {
        if matches!(inline, Inline::Enter(_) | Inline::Exit(_)) {
            continue;
        }
        assert_eq!(range.start, prev, "a gap opened at {range:?}");
        prev = range.end;
    }
    // The paragraph's own range runs to the end of the line; the spans stop at the last character.
    assert_eq!(prev, source.trim_end().len());
    assert!(prev <= para.range.end);
}

#[test]
fn autolink_handles_non_ascii_neighbours() {
    // The path scan walks bytes, so it must stop on a char boundary whatever is around it.
    let source = "Voir https://exemple.fr/café — fin, ou https://exemple.fr/thé.\n";
    let d = parse(source);
    let l = links(&d.blocks[0].spans);
    assert_eq!(l.len(), 2);
    assert_eq!(&d.source[l[0].0.clone()], "https://exemple.fr/café");
    assert_eq!(&d.source[l[1].0.clone()], "https://exemple.fr/thé");
}

#[test]
fn autolink_reaches_into_every_container() {
    let d = parse(
        "- item https://a.com/one\n\n\
         > quote https://b.com/two\n\n\
         | head https://c.com/three |\n|---|\n| cell https://d.com/four |\n\n\
         Body[^n]\n\n\
         [^n]: note https://e.com/five\n",
    );
    let mut dests = all_dests(&d.blocks);
    dests.sort();
    assert_eq!(
        dests,
        vec![
            "https://a.com/one",
            "https://b.com/two",
            "https://c.com/three",
            "https://d.com/four",
            "https://e.com/five",
        ]
    );
}

/// A real document off disk — the 150KB case §5.1 names, and the one that catches the constructs a
/// hand-written sample never has. Skips cleanly when the base checkout is absent.
#[test]
fn a_real_document_parses_sanely() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let candidates = [
        "/_docs/tech/decisions.md",
        "/_docs/inbox/markdown-improvement-proposal.md",
    ];
    let Some(source) = candidates
        .iter()
        .find_map(|p| std::fs::read_to_string(format!("{root}{p}")).ok())
    else {
        eprintln!("no document under _docs — skipping");
        return;
    };
    assert!(source.len() > 20_000, "expected a large document");

    // Not an assertion — a test-harness timing is not a frame budget. It is printed because §5.1
    // wants a number for the 150KB case and this is where one is free. Measured at ~3.7ms release,
    // ~29ms debug for 350KB.
    let t = std::time::Instant::now();
    let d = parse(&source);
    eprintln!(
        "parsed {}KB into {} blocks in {:?}",
        source.len() / 1024,
        d.blocks.len(),
        t.elapsed()
    );
    assert!(d.blocks.len() > 100, "got {} blocks", d.blocks.len());

    // Ordered, disjoint, in bounds — the invariant, over real input.
    let mut prev_end = 0;
    for b in &d.blocks {
        assert!(b.range.start >= prev_end, "block {} overlaps", b.ix);
        assert!(b.range.end <= source.len());
        prev_end = b.range.end;
    }

    // Every block still re-parses to itself.
    for b in &d.blocks {
        let re = parse(&source[b.range.clone()]);
        assert_eq!(
            re.blocks.len(),
            1,
            "block {} ({}) split on re-parse",
            b.ix,
            kind_name(&b.kind)
        );
        assert_eq!(
            kind_name(&re.blocks[0].kind),
            kind_name(&b.kind),
            "block {} changed kind",
            b.ix
        );
    }

    // The tree's documents all start with front matter and are heavy in headings and tables.
    assert!(matches!(d.blocks[0].kind, BlockKind::FrontMatter { .. }));
    let headings = d.headings();
    assert!(headings.len() > 5);
    let mut slugs: Vec<_> = headings.iter().map(|h| h.slug.clone()).collect();
    let n = slugs.len();
    slugs.sort();
    slugs.dedup();
    assert_eq!(slugs.len(), n, "slugs are unique across a real document");

    // And an edit in the middle still splices a handful of rows, not the file.
    let mid = source.len() / 2;
    let cut = (mid..source.len())
        .find(|i| source.is_char_boundary(*i))
        .unwrap();
    let edited = format!("{}{}{}", &source[..cut], "xyzzy ", &source[cut..]);
    let mut d = d;
    let delta = d.reparse(&edited);
    assert!(
        delta.old_len < 5 && delta.new_len < 5,
        "a one-word edit spliced {} rows",
        delta.old_len.max(delta.new_len)
    );
}

// --- search -----------------------------------------------------------------------------------
//
// The load-bearing claim here is the mapping: a match found in the *rendered* text has to come back
// as source bytes the viewer can scroll to and highlight. So nearly every assertion is on
// `&source[m.range]`, not on the match count.

/// Matches as `(root block, the source bytes they name, rendered?)`.
fn hits(d: &Document, query: &str, opts: FindOptions) -> Vec<(usize, String, bool)> {
    d.find(query, opts)
        .into_iter()
        .map(|m| {
            (
                m.root_block,
                d.source[m.range].to_string(),
                m.in_rendered_text,
            )
        })
        .collect()
}

fn sensitive() -> FindOptions {
    FindOptions {
        case_sensitive: true,
        ..FindOptions::default()
    }
}

fn whole_word() -> FindOptions {
    FindOptions {
        whole_word: true,
        ..FindOptions::default()
    }
}

#[test]
fn find_matches_across_inline_emphasis() {
    let d = parse("foo *bar* baz\n");
    // The rendered text is `foo bar baz`, so the query spans the emphasis without seeing it.
    assert_eq!(
        hits(&d, "foo bar", FindOptions::default()),
        vec![(0, "foo *bar".to_string(), true)],
        "the tightest source span covering both ends of the match"
    );
    // And the syntax is not content: `*` was never in the rendered string.
    assert!(d.find("*bar*", FindOptions::default()).is_empty());
    assert!(d.find("o *b", FindOptions::default()).is_empty());
}

#[test]
fn find_maps_a_soft_break_and_inline_code_back_to_bytes() {
    let d = parse("foo\nbar\n");
    assert_eq!(
        hits(&d, "foo bar", FindOptions::default()),
        vec![(0, "foo\nbar".to_string(), true)],
        "a soft break renders as one space and maps back to the newline"
    );

    let d = parse("A `needle()` call.\n");
    assert_eq!(
        hits(&d, "needle()", FindOptions::default()),
        vec![(0, "needle()".to_string(), true)],
        "inline code maps to its payload, backticks excluded"
    );
}

#[test]
fn find_reaches_a_table_cell_and_names_the_root_table() {
    let d = parse("# H\n\n| a | b |\n|---|---|\n| a needle here | d |\n");
    assert!(matches!(d.blocks[1].kind, BlockKind::Table { .. }));
    assert_eq!(
        hits(&d, "needle", FindOptions::default()),
        vec![(1, "needle".to_string(), true)],
        "a cell hit is attributed to the root table row"
    );
}

#[test]
fn find_reaches_a_nested_list_item_and_names_the_root_list() {
    let d = parse("Intro.\n\n- outer\n  - inner *needle* text\n");
    assert!(matches!(d.blocks[1].kind, BlockKind::List { .. }));
    assert_eq!(
        hits(&d, "inner needle", FindOptions::default()),
        vec![(1, "inner *needle".to_string(), true)],
        "a nested item's sibling-local ix never leaks; the root list row does"
    );
}

#[test]
fn find_reaches_quotes_and_footnote_definitions() {
    let d = parse("> quoted needle\n\nBody[^n]\n\n[^n]: note needle\n");
    assert!(matches!(d.blocks[0].kind, BlockKind::Quote { .. }));
    assert!(matches!(
        d.blocks[2].kind,
        BlockKind::FootnoteDefinition { .. }
    ));
    assert_eq!(
        hits(&d, "needle", FindOptions::default()),
        vec![
            (0, "needle".to_string(), true),
            (2, "needle".to_string(), true),
        ]
    );
}

#[test]
fn find_is_case_insensitive_by_default_and_sensitive_on_request() {
    let d = parse("Needle in a haystack.\n");
    assert_eq!(d.find("needle", FindOptions::default()).len(), 1);
    assert_eq!(d.find("NEEDLE", FindOptions::default()).len(), 1);
    assert!(d.find("needle", sensitive()).is_empty());
    assert_eq!(
        hits(&d, "Needle", sensitive()),
        vec![(0, "Needle".to_string(), true)]
    );
}

#[test]
fn find_whole_word_rejects_a_substring() {
    let d = parse("a needle and a needless thing, needle_case too\n");
    assert_eq!(
        d.find("needle", FindOptions::default()).len(),
        3,
        "substring matches by default"
    );
    let ws = d.find("needle", whole_word());
    assert_eq!(ws.len(), 1, "only the standalone word survives");
    assert_eq!(&d.source[ws[0].range.clone()], "needle");
    assert_eq!(ws[0].range.start, d.source.find("needle").unwrap());
}

#[test]
fn find_matches_markdown_syntax_only_in_raw_mode() {
    let d = parse("## A heading\n\nfoo *bar*\n");
    for q in ["*bar*", "## ", "#"] {
        assert!(
            d.find(q, FindOptions::default()).is_empty(),
            "{q:?} is syntax, not rendered content"
        );
    }
    assert_eq!(
        hits(&d, "*bar*", FindOptions::raw()),
        vec![(1, "*bar*".to_string(), false)]
    );
    assert_eq!(
        hits(&d, "## ", FindOptions::raw()),
        vec![(0, "## ".to_string(), false)]
    );
}

#[test]
fn find_snaps_an_inexact_span_to_its_boundaries() {
    // `&amp;` is one rendered byte from five source bytes, so offsets inside it cannot be computed.
    let d = parse("a &amp; b needle\n");
    assert_eq!(
        hits(&d, "a & b", FindOptions::default()),
        vec![(0, "a &amp; b".to_string(), true)],
        "the end snaps forward past the whole entity"
    );
    assert_eq!(
        hits(&d, "& b", FindOptions::default()),
        vec![(0, "&amp; b".to_string(), true)],
        "the start snaps back to the span's first byte"
    );
    // A match clear of the entity is still exact.
    assert_eq!(
        hits(&d, "needle", FindOptions::default()),
        vec![(0, "needle".to_string(), true)]
    );
}

#[test]
fn find_scans_spanless_blocks_over_their_own_bytes() {
    let d = doc();
    let ms = d.find("println", FindOptions::default());
    assert_eq!(ms.len(), 1);
    assert!(matches!(
        d.blocks[ms[0].root_block].kind,
        BlockKind::Code { .. }
    ));
    assert_eq!(&d.source[ms[0].range.clone()], "println");
    assert!(
        !ms[0].in_rendered_text,
        "a code block has no inline stream, so the hit is honest about being raw"
    );
}

#[test]
fn find_ranges_are_exact_bytes_when_the_mapping_is() {
    let d = doc();
    for q in ["paragraph", "Sample", "second", "Notes", "strikethrough"] {
        let ms = d.find(q, FindOptions::default());
        assert!(!ms.is_empty(), "{q:?} not found");
        for m in &ms {
            assert_eq!(
                d.source[m.range.clone()].to_lowercase(),
                q.to_lowercase(),
                "{q:?} mapped to the wrong bytes in block {}",
                m.root_block
            );
            assert!(m.root_block < d.blocks.len());
            assert!(d.blocks[m.root_block].range.start <= m.range.start);
            assert!(m.range.end <= d.blocks[m.root_block].range.end);
        }
    }
    // Source order, and nothing for an empty query.
    let ms = d.find("e", FindOptions::default());
    assert!(ms.windows(2).all(|w| w[0].range.start <= w[1].range.start));
    assert!(d.find("", FindOptions::default()).is_empty());
}

#[test]
fn find_heading_resolves_text_and_slug() {
    let d = doc();
    let title = d.block_for_slug("the-title").unwrap();
    assert_eq!(d.find_heading("The title"), Some(title), "by text");
    assert_eq!(d.find_heading("the title"), Some(title), "case-insensitive");
    assert_eq!(d.find_heading("#the-title"), Some(title), "by anchor");
    assert_eq!(d.find_heading("the-title"), Some(title), "by slug");
    assert_eq!(d.find_heading("titl"), Some(title), "by prefix of the text");

    // SAMPLE has two `## Notes`; the deduped slug reaches the second, plain text the first.
    let first = d.block_for_slug("notes").unwrap();
    let second = d.block_for_slug("notes-1").unwrap();
    assert_ne!(first, second);
    assert_eq!(d.find_heading("Notes"), Some(first));
    assert_eq!(d.find_heading("notes-1"), Some(second));

    assert_eq!(d.find_heading("no such section"), None);
    assert_eq!(d.find_heading("  "), None);
}
