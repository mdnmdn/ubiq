//! The outline panel: the definitions in the buffer on screen, and a click that goes to one.
//!
//! **It is answered entirely from the open buffer.** No host message, no index, no project. A file
//! dragged in from outside every project gets its outline the same as one the tree opened, because
//! the only input is text plus an extension. That is the whole point of keeping it here rather than
//! asking the coordinator for it.
//!
//! Each grammar ships the query that names its own definitions — `TAGS_QUERY`, the same one a
//! tags-file generator uses. Markdown is the exception: it ships none, so `languages/markdown`
//! holds the two patterns that pick out its headings.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, uniform_list,
};
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::app::AppState;
use crate::state::editor::{Subject, tab_key};
use crate::state::nav::{Destination, Locus, View, range_for};
use crate::theme;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{mono, panel, row_height};

/// The heading query `tree-sitter-md` does not ship. See the file's own comment.
const MARKDOWN_TAGS: &str = include_str!("languages/markdown/tags.scm");

/// Buffers past this are not parsed. gpui-component gives up on its own synchronous highlight
/// parse at 256 KB; the outline runs off the main thread and can afford twice that, but a panel
/// that stalls the window on a generated file is worse than a panel that says nothing.
const MAX_BYTES: usize = 512 * 1024;

/// The most rows one file contributes. A generated file with tens of thousands of definitions is a
/// list nobody scrolls, and building the elements for it costs a frame.
const MAX_DEFS: usize = 2000;

/// What a definition is, as far as a list of them goes. Taken from the capture name's suffix after
/// `definition.`, which is the only vocabulary the grammars agree on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefKind {
    Function,
    Method,
    Class,
    Interface,
    Type,
    Module,
    Constant,
    Macro,
    Property,
    Heading,
    /// A suffix this build does not know. Grammars add captures between releases, and a definition
    /// drawn under a blank sigil is better than one silently dropped.
    Other,
}

impl DefKind {
    fn of(capture: &str) -> Self {
        match capture.rsplit('.').next().unwrap_or_default() {
            "function" => DefKind::Function,
            "method" => DefKind::Method,
            "class" | "struct" | "enum" | "union" => DefKind::Class,
            "interface" | "trait" | "protocol" => DefKind::Interface,
            "type" => DefKind::Type,
            "module" | "namespace" | "package" => DefKind::Module,
            "constant" | "field" | "variable" => DefKind::Constant,
            "macro" => DefKind::Macro,
            "property" => DefKind::Property,
            "heading" => DefKind::Heading,
            _ => DefKind::Other,
        }
    }

    /// The one or two characters drawn in the gutter beside the name. A sigil rather than an icon:
    /// the list is monospaced, so a fixed-width mark keeps the names in a column.
    fn sigil(self) -> &'static str {
        match self {
            DefKind::Function => "fn",
            DefKind::Method => "m",
            DefKind::Class => "C",
            DefKind::Interface => "I",
            DefKind::Type => "T",
            DefKind::Module => "mod",
            DefKind::Constant => "c",
            DefKind::Macro => "!",
            DefKind::Property => "p",
            DefKind::Heading => "#",
            DefKind::Other => "·",
        }
    }
}

/// One definition, named and placed. The line is one-based, because that is what a `Locus` carries
/// and what the editor's gutter shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Def {
    pub name: String,
    pub kind: DefKind,
    pub line: u32,
}

/// TypeScript's own tags query names only what TypeScript adds — signatures, interfaces, abstract
/// classes, modules — and leaves plain functions and classes to JavaScript's, which is the
/// convention every consumer of these queries follows. Joined once and kept, because a query source
/// is looked up on every parse and on every frame the panel draws.
fn typescript_tags() -> &'static str {
    static JOINED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    JOINED.get_or_init(|| {
        format!(
            "{}\n{}",
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY
        )
    })
}

/// The grammar and the query that names definitions in it, for a file extension.
///
/// A local match rather than a detour through `FileLanguage`: that enum answers "what does the
/// highlighter call this", and several of its arms — Json, Yaml, Toml, Diff, Plain — have no tags
/// query at all, so mapping through it would only have to be unmapped again here.
fn grammar(ext: &str) -> Option<(Language, &'static str)> {
    let (language, query) = match ext {
        "rs" => (tree_sitter_rust::LANGUAGE, tree_sitter_rust::TAGS_QUERY),
        "py" | "pyi" => (tree_sitter_python::LANGUAGE, tree_sitter_python::TAGS_QUERY),
        "js" | "jsx" | "mjs" | "cjs" => (
            tree_sitter_javascript::LANGUAGE,
            tree_sitter_javascript::TAGS_QUERY,
        ),
        "ts" | "mts" | "cts" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            typescript_tags(),
        ),
        "tsx" => (tree_sitter_typescript::LANGUAGE_TSX, typescript_tags()),
        "go" => (tree_sitter_go::LANGUAGE, tree_sitter_go::TAGS_QUERY),
        "java" => (tree_sitter_java::LANGUAGE, tree_sitter_java::TAGS_QUERY),
        "c" | "h" => (tree_sitter_c::LANGUAGE, tree_sitter_c::TAGS_QUERY),
        "cs" => (
            tree_sitter_c_sharp::LANGUAGE,
            tree_sitter_c_sharp::TAGS_QUERY,
        ),
        "swift" => (tree_sitter_swift::LANGUAGE, tree_sitter_swift::TAGS_QUERY),
        "md" | "markdown" => (tree_sitter_md::LANGUAGE, MARKDOWN_TAGS),
        _ => return None,
    };
    Some((Language::new(language), query))
}

/// The definitions in one buffer, in the order they appear.
///
/// Empty for an extension with no grammar, for a buffer too big to be worth parsing, and for a
/// buffer that simply has none — all three are the same answer to the panel, which draws its empty
/// page either way.
pub fn extract(text: &str, ext: &str) -> Vec<Def> {
    if text.len() > MAX_BYTES {
        return Vec::new();
    }
    let Some((language, source)) = grammar(&ext.to_ascii_lowercase()) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(&language, source) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    // ponytail: second parse of the same buffer — gpui-component owns a tree we cannot reach;
    // delete this the day InputHighlighter grows a tree accessor.
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };

    let names = query.capture_names();
    let bytes = text.as_bytes();
    let mut defs = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if defs.len() >= MAX_DEFS {
            break;
        }
        // A tags pattern captures the whole definition and, inside it, the identifier. Both have to
        // be present: a `@definition.*` with no `@name` beside it has nothing to put in a row.
        let Some(kind) = m
            .captures
            .iter()
            .map(|c| names[c.index as usize])
            .find(|name| name.starts_with("definition."))
            .map(DefKind::of)
        else {
            continue;
        };
        let Some(name) = m
            .captures
            .iter()
            .find(|c| names[c.index as usize] == "name")
        else {
            continue;
        };
        let Ok(text) = name.node.utf8_text(bytes) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        defs.push(Def {
            name: text.to_string(),
            kind,
            line: name.node.start_position().row as u32 + 1,
        });
    }

    // The query walks the tree, not the file, so matches arrive in pattern order rather than
    // reading order. Every consumer — the list and `enclosing` both — wants reading order.
    defs.sort_by_key(|def| def.line);
    // Several grammars name the same thing twice — Rust's tags query calls a method in an `impl`
    // both `definition.method` and `definition.function` — and one thing is one row. The sort is
    // stable, so the surviving kind is the one the grammar listed first, which is the specific one.
    defs.dedup_by(|a, b| a.line == b.line && a.name == b.name);
    defs
}

/// The definition a cursor line falls in: the last one at or above it.
///
/// A binary search over a list already sorted by line, rather than a second parse. It is an
/// approximation — a line between two definitions belongs to the one above it, whether or not it is
/// really inside — and that is the right answer for a breadcrumb.
pub fn enclosing(defs: &[Def], line: u32) -> Option<&Def> {
    let at = defs.partition_point(|def| def.line <= line);
    at.checked_sub(1).map(|index| &defs[index])
}

/// The panel. Read-only: the list is `AppState`'s, refreshed on a debounce after the last edit —
/// see `AppState::schedule_outline`.
pub fn render(app: &AppState, _window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(file) = app.editor(cx).and_then(|editor| editor.active_file()) else {
        return panel()
            .child(empty_panel("No file open."))
            .into_any_element();
    };
    if app.outline.is_empty() {
        let note = if grammar(&extension(&file.path)).is_some() {
            "No definitions in this file."
        } else {
            "No outline for this file type."
        };
        return panel().child(empty_panel(note)).into_any_element();
    }

    let project = app.project(cx);
    let key = tab_key(&file.path, Subject::File);
    // The list follows the project's own text size, the way the search results and the tree do.
    let font = app.content_font_size_or_default(cx) - 0.5;
    let row = row_height(font);
    // The definition the caret is in, so the row the user is standing in is lit.
    let here = app
        .cursor_line_column(cx)
        .and_then(|(line, _)| enclosing(&app.outline, line))
        .map(|def| def.line);

    // The definitions themselves stay where they are: the list reads them back out of the window
    // when it builds the rows on screen, so a two-thousand-definition file costs a viewport.
    let count = app.outline.len();
    let view = cx.entity();

    panel()
        .child(
            uniform_list("outline-list", count, move |range, window, cx| {
                let defs = &view.read(cx).outline;
                range
                    .filter_map(|index| {
                        let def = defs.get(index)?;
                        let line = def.line;
                        let key = key.clone();
                        Some(
                            div()
                                .id(gpui::ElementId::Name(format!("outline-{line}").into()))
                                .cursor_pointer()
                                .when(here == Some(line), |this| this.bg(theme::selected()))
                                .hover(|this| this.bg(theme::hover()))
                                .on_click(window.listener_for(&view, move |this, _, _, cx| {
                                    this.goto_outline(project, &key, line, cx)
                                }))
                                .h(px(row))
                                .px_3()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_2()
                                .child(
                                    mono(def.kind.sigil(), theme::text_faint())
                                        .text_size(px(font - 1.))
                                        .flex_none()
                                        .w(px(font * 2.2)),
                                )
                                .child(
                                    mono(def.name.clone(), theme::text())
                                        .text_size(px(font))
                                        .flex_1()
                                        .min_w(px(0.))
                                        .truncate(),
                                )
                                .child(
                                    mono(format!("{line}"), theme::text_faint())
                                        .text_size(px(font - 1.))
                                        .flex_none(),
                                )
                                .into_any_element(),
                        )
                    })
                    .collect::<Vec<AnyElement>>()
            })
            .flex_1()
            .min_h(px(0.)),
        )
        .into_any_element()
}

/// A path's extension, lowercased by the caller when it matters. Empty for a file without one,
/// which no grammar answers to.
pub fn extension(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

impl AppState {
    /// Follow an outline row.
    ///
    /// With a project on screen this is exactly the search panel's jump — one `Destination` through
    /// [`AppState::navigate`], so the row is written into the history like every other arrival.
    /// Without one there is no `ProjectId` to build a destination from, and none is needed: the row
    /// names a spot in the buffer already on screen, so moving the caret *is* the jump.
    fn goto_outline(
        &mut self,
        project: Option<ubiq_proto::ids::ProjectId>,
        key: &str,
        line: u32,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = project {
            self.navigate(
                Destination {
                    project,
                    view: View::Ide {
                        key: key.to_string(),
                    },
                    locus: Some(Locus::Line { line }),
                },
                cx,
            );
            return;
        }
        let Some(buffer) = self
            .editor(cx)
            .and_then(|editor| editor.active_file())
            .and_then(|file| file.buffer())
            .cloned()
        else {
            return;
        };
        let text = buffer.read(cx).value().to_string();
        if let Some(range) = range_for(&text, &Locus::Line { line }) {
            buffer.update(cx, |state, cx| state.set_selected_range(range, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_definitions_are_named_placed_and_kinded() {
        let source = "\
struct Widget {
    size: u32,
}

impl Widget {
    fn new() -> Self {
        Widget { size: 0 }
    }
}

fn main() {}
";
        let defs = extract(source, "rs");
        let seen: Vec<_> = defs
            .iter()
            .map(|def| (def.name.as_str(), def.kind, def.line))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("Widget", DefKind::Class, 1),
                ("new", DefKind::Method, 6),
                ("main", DefKind::Function, 11),
            ]
        );
    }

    #[test]
    fn markdown_yields_its_headings() {
        let source = "\
# Title

Some prose.

## A section

Underlined
==========
";
        let defs = extract(source, "md");
        let seen: Vec<_> = defs
            .iter()
            .map(|def| (def.name.as_str(), def.kind, def.line))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("Title", DefKind::Heading, 1),
                ("A section", DefKind::Heading, 5),
                ("Underlined", DefKind::Heading, 7),
            ]
        );
    }

    #[test]
    fn enclosing_is_the_definition_above_the_line() {
        let defs = extract("fn one() {}\n\nfn two() {}\n", "rs");
        assert_eq!(defs.len(), 2);
        assert_eq!(enclosing(&defs, 1).map(|d| d.name.as_str()), Some("one"));
        assert_eq!(enclosing(&defs, 2).map(|d| d.name.as_str()), Some("one"));
        assert_eq!(enclosing(&defs, 3).map(|d| d.name.as_str()), Some("two"));
        assert_eq!(enclosing(&defs, 99).map(|d| d.name.as_str()), Some("two"));
        // Nothing is above the first definition, so a line before it is in nothing.
        let below = extract("\n\nfn late() {}\n", "rs");
        assert_eq!(enclosing(&below, 1), None);
    }

    #[test]
    fn an_unknown_extension_is_an_empty_outline_not_an_error() {
        assert!(extract("anything at all", "xyz").is_empty());
        assert!(extract("", "").is_empty());
    }

    /// Every grammar is loaded once, because a version skew between two `tree-sitter` crates in
    /// the tree shows up as a runtime `LanguageError` and never at compile time.
    #[test]
    fn every_grammar_loads_and_answers() {
        let cases = [
            ("rs", "fn a() {}\n"),
            ("py", "def a():\n    pass\n"),
            ("js", "function a() {}\n"),
            ("ts", "function a(): void {}\n"),
            ("tsx", "function a() { return <p/>; }\n"),
            ("go", "package p\nfunc a() {}\n"),
            ("java", "class A { void a() {} }\n"),
            ("c", "void a(void) {}\n"),
            ("cs", "class A { void a() {} }\n"),
            ("swift", "func a() {}\n"),
            ("md", "# a\n"),
        ];
        for (ext, source) in cases {
            assert!(
                !extract(source, ext).is_empty(),
                "{ext} produced no definitions"
            );
        }
    }

    #[test]
    fn an_oversized_buffer_is_not_parsed() {
        let huge = "fn a() {}\n".repeat(MAX_BYTES / 10 + 1);
        assert!(huge.len() > MAX_BYTES);
        assert!(extract(&huge, "rs").is_empty());
    }
}
