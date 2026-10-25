//! Images in a help page, loaded from the unpacked bundle.
//!
//! The library's text view hands every markdown image to GPUI as a URI — inline `![alt](x)`,
//! inline `<img src>` and the standalone image block alike — and GPUI loads a URI through the
//! application's HTTP client, of which Ubiq installs none. Two halves close that gap, both scoped
//! to the help root the host named:
//!
//! - [`localise`] rewrites a page's page-relative image paths (`../img/ui/x.png`) into
//!   `file://localhost/…` URIs under that root, when the page is read. A path that climbs out of
//!   the root, an absolute path and anything with a scheme are left as written.
//! - [`HelpImages`] is the HTTP client: it answers a `file:` URI under the root and nothing else.
//!   `localhost` is spelled out because GPUI's request type refuses a `file:///` URI with an empty
//!   authority.
//!
//! The same read-time pass folds each soft line break in a paragraph to a space, because the
//! library's text view draws one as a hard break (`G419`); a page wrapped in its source then flows.
//!
//! The rewrite works on the parsed tree's source spans, so an image written inside a code block or
//! a code span — an authoring example — is never touched.

use std::future::Future;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::OnceLock;

use gpui::http_client::{AsyncBody, HttpClient, Request, Response, Url, http};
use markdown::mdast::Node;

/// Every image in `source`, written relative to the page at `page_path` (a path inside the bundle,
/// `/`-separated), rewritten to a `file:` URI under `root`. Everything else is left byte for byte.
pub(super) fn localise(source: &str, root: &Path, page_path: &str) -> String {
    let options = markdown::ParseOptions {
        constructs: markdown::Constructs {
            frontmatter: true,
            ..markdown::Constructs::gfm()
        },
        ..markdown::ParseOptions::gfm()
    };
    let Ok(tree) = markdown::to_mdast(source, &options) else {
        return source.to_string();
    };
    let folder = page_path.rsplit_once('/').map_or("", |(folder, _)| folder);
    let resolve = |url: &str| file_uri(root, folder, url);

    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    collect(&tree, source, &resolve, false, &mut edits);
    edits.sort_by_key(|(range, _)| range.start);

    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for (range, replacement) in edits {
        if range.start < at {
            continue;
        }
        out.push_str(&source[at..range.start]);
        out.push_str(&replacement);
        at = range.end;
    }
    out.push_str(&source[at..]);
    out
}

/// The edits one subtree asks for: an image's destination, the `src` of an `<img>` tag, and each
/// soft line break in a paragraph's text. `in_paragraph` is set below a `Paragraph`.
fn collect(
    node: &Node,
    source: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
    in_paragraph: bool,
    edits: &mut Vec<(Range<usize>, String)>,
) {
    let span = node
        .position()
        .map(|position| position.start.offset..position.end.offset)
        .filter(|span| source.get(span.clone()).is_some());
    match (node, span) {
        (Node::Image(image), Some(span)) => {
            // The destination as written, found right after the `](` that opens it. A destination
            // written in some other spelling — escaped, in angle brackets — is left alone.
            let written = &source[span.clone()];
            if let Some(uri) = resolve(&image.url)
                && let Some(cut) = written.find(&format!("]({}", image.url))
            {
                let start = span.start + cut + 2;
                edits.push((start..start + image.url.len(), uri));
            }
        }
        (Node::Html(_), Some(span)) => {
            let written = &source[span.clone()];
            for found in img_src().captures_iter(written) {
                let Some(url) = found.get(1).or_else(|| found.get(2)) else {
                    continue;
                };
                if let Some(uri) = resolve(url.as_str()) {
                    edits.push((span.start + url.start()..span.start + url.end(), uri));
                }
            }
        }
        (Node::Text(_), Some(span)) if in_paragraph => soft_breaks(source, span, edits),
        _ => {}
    }
    let in_paragraph = in_paragraph || matches!(node, Node::Paragraph(_));
    if let Some(children) = node.children() {
        for child in children {
            collect(child, source, resolve, in_paragraph, edits);
        }
    }
}

/// Each newline inside one paragraph text node's `span`, with the spaces before it and the next
/// line's indent and `>` markers, folded to one space (`G419`). A hard break (two trailing spaces,
/// a backslash) is a `Break` node of its own, and code spans and inline HTML are not `Text`, so
/// none of them reaches here. Inside a paragraph a line cannot open with a `>` of its own — that
/// would start a block quote — so every `>` after a newline is a container marker.
fn soft_breaks(source: &str, span: Range<usize>, edits: &mut Vec<(Range<usize>, String)>) {
    let bytes = source.as_bytes();
    let mut at = span.start;
    while let Some(found) = source[at..span.end].find('\n') {
        let newline = at + found;
        let mut start = newline;
        while start > span.start && matches!(bytes[start - 1], b' ' | b'\t' | b'\r') {
            start -= 1;
        }
        let mut end = newline + 1;
        while end < span.end && matches!(bytes[end], b' ' | b'\t' | b'>') {
            end += 1;
        }
        edits.push((start..end, " ".to_string()));
        at = end;
    }
}

/// The `src` of an `<img>` tag, double- or single-quoted.
fn img_src() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"(?i)<img\b[^>]*?\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
            .expect("a valid pattern")
    })
}

/// `url`, relative to `folder` inside `root`, as a `file://localhost/…` URI — or `None` for
/// anything that is not page-relative or climbs out of the root.
fn file_uri(root: &Path, folder: &str, url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let has_scheme = path.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if path.is_empty() || path.starts_with(['/', '\\']) || has_scheme {
        return None;
    }
    let mut stack: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            part => stack.push(part),
        }
    }
    let mut file = root.to_path_buf();
    file.extend(&stack);
    let mut uri = Url::from_file_path(&file).ok()?;
    uri.set_host(Some("localhost")).ok()?;
    Some(uri.to_string())
}

/// The window's HTTP client: the help bundle's files, and nothing else.
///
/// Installed when the host names the help root (`HelpReady`). Any other URI — the web, a `file:`
/// path outside the root — is refused, so a markdown document cannot read the disk through an
/// image, and the library's "no implicit filesystem access" stance holds everywhere but here.
pub(super) struct HelpImages {
    root: PathBuf,
}

impl HelpImages {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        }
    }
}

type Answer = Pin<Box<dyn Future<Output = anyhow::Result<Response<AsyncBody>>> + Send + 'static>>;

impl HttpClient for HelpImages {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(&self, request: Request<AsyncBody>) -> Answer {
        let root = self.root.clone();
        let uri = request.uri().to_string();
        Box::pin(async move {
            let file = Url::parse(&uri)
                .ok()
                .filter(|url| url.scheme() == "file")
                .and_then(|url| url.to_file_path().ok())
                .ok_or_else(|| anyhow::anyhow!("no HTTP client: only help images load ({uri})"))?;
            let bytes = file
                .canonicalize()
                .ok()
                .filter(|file| file.starts_with(&root))
                .and_then(|file| std::fs::read(file).ok());
            let (status, body) = match bytes {
                Some(bytes) => (http::StatusCode::OK, bytes),
                None => (http::StatusCode::NOT_FOUND, Vec::new()),
            };
            Ok(Response::builder()
                .status(status)
                .body(AsyncBody::from(body))?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(path: &str) -> String {
        let mut url = Url::from_file_path(path).expect("absolute");
        url.set_host(Some("localhost")).expect("a host");
        url.to_string()
    }

    #[test]
    fn page_relative_images_resolve_under_the_root() {
        let root = Path::new("/r/help");
        let page = "---\nid: x\n---\n\nText ![a](../img/a.png) and <img src=\"../img/b.png\" width=\"16\">.\n\n![c](c.png)\n";
        let out = localise(page, root, "guides/page.md");
        assert!(out.contains(&format!("![a]({})", uri("/r/help/img/a.png"))));
        assert!(out.contains(&format!(
            "<img src=\"{}\" width=\"16\">",
            uri("/r/help/img/b.png")
        )));
        assert!(out.contains(&format!("![c]({})", uri("/r/help/guides/c.png"))));
        assert!(out.starts_with("---\nid: x\n---\n"));
    }

    #[test]
    fn code_absolute_schemes_and_escapes_are_left_alone() {
        let root = Path::new("/r/help");
        let page = "`![a](x.png)`\n\n```\n<img src=\"x.png\">\n```\n\n![b](https://e/x.png) ![c](/x.png) ![d](../../x.png)\n";
        assert_eq!(localise(page, root, "page.md"), page);
    }

    #[test]
    fn soft_breaks_in_a_paragraph_fold_to_a_space() {
        let root = Path::new("/r/help");
        let page = "One\ntwo *three\nfour* end.\n\n- item\n  wrapped\n\n> quoted\n> line\n";
        assert_eq!(
            localise(page, root, "page.md"),
            "One two *three four* end.\n\n- item wrapped\n\n> quoted line\n"
        );
    }

    #[test]
    fn hard_breaks_code_html_tables_and_headings_keep_their_newlines() {
        let root = Path::new("/r/help");
        let page = "a  \nb\\\nc\n\n`x\ny`\n\n<span>\nz</span>\n\n```\nq\nr\n```\n\n| h |\n|---|\n| v |\n\nTitle\nmore\n=====\n";
        let out = localise(page, root, "page.md");
        assert!(out.starts_with(
            "a  \nb\\\nc\n\n`x\ny`\n\n<span>\nz</span>\n\n```\nq\nr\n```\n\n| h |\n|---|\n| v |\n"
        ));
        assert!(out.ends_with("Title\nmore\n=====\n"));
    }

    #[test]
    fn a_uri_with_an_empty_authority_would_not_reach_the_client() {
        assert!("file:///r/x.png".parse::<http::Uri>().is_err());
        assert!(uri("/r/x.png").parse::<http::Uri>().is_ok());
    }
}
