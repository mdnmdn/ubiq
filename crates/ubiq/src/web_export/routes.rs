//! Request handling: `/<slug>/<rest...>` resolves against the project root registered for
//! `<slug>`, classifies by extension, and responds. `/_assets/...` serves the doc-viewer chrome.

use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use tiny_http::{Header, Request, Response};

use super::assets;
use super::server::{ProjectEntry, SharedRegistry};

pub(super) fn handle(request: Request, registry: &SharedRegistry) {
    // The query string carries `_search`'s `q` param, so unlike the old `.split('?').next()` this
    // has to keep both halves around instead of throwing the query away.
    let mut url_parts = request.url().splitn(2, '?');
    let raw_path = url_parts.next().unwrap_or("/").to_string();
    let raw_query = url_parts.next().unwrap_or("").to_string();
    let decoded = percent_decode(&raw_path);
    let mut segments: Vec<String> = decoded
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    if segments.first().map(String::as_str) == Some("_assets") {
        serve_asset(request, segments.get(1).map(String::as_str));
        return;
    }

    if segments.is_empty() {
        let _ = request.respond(not_found());
        return;
    }
    let slug = segments.remove(0);

    let project = registry.lock().unwrap().lookup(&slug);
    let Some(project) = project else {
        let _ = request.respond(not_found());
        return;
    };

    let rest: Vec<&str> = segments.iter().map(String::as_str).collect();
    if rest == ["_search"] {
        serve_search(request, &project, &raw_query);
        return;
    }
    match resolve_path(&project.root, &rest) {
        Some(resolved) => serve_path(request, &slug, &project, &resolved, &raw_query),
        None => {
            let _ = request.respond(not_found());
        }
    }
}

// --- Path resolution ---

/// Joins `rest` onto `root` and refuses anything that doesn't land back inside `root` — a `..`
/// segment is rejected outright, and the joined path is canonicalized and re-checked against the
/// canonicalized root as defense against symlink escapes.
fn resolve_path(root: &Path, rest: &[&str]) -> Option<PathBuf> {
    let mut candidate = root.to_path_buf();
    for segment in rest {
        // A leading dot also catches `.git`, `.env` and the like — never servable, matching the
        // indexer's own walk (`render_nav_tree` already skips them via `ignore`'s hidden-file
        // default, but that only hides them from the listing; a direct URL still has to be
        // refused here).
        if *segment == ".." || segment.is_empty() || segment.starts_with('.') {
            return None;
        }
        candidate.push(segment);
    }
    if !candidate.exists() {
        return None;
    }
    let root_canon = root.canonicalize().ok()?;
    let candidate_canon = candidate.canonicalize().ok()?;
    candidate_canon
        .starts_with(&root_canon)
        .then_some(candidate_canon)
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .replace('\\', "/")
}

// --- Dispatch by kind ---

/// ~256 KiB: past this, an unrecognised extension is never read at all — only its size is shown.
/// Under it, one read decides text-or-not by sniffing rather than guessing from the name.
const UNKNOWN_TEXT_SNIFF_LIMIT: u64 = 256 * 1024;

fn serve_path(
    request: Request,
    slug: &str,
    project: &ProjectEntry,
    resolved: &Path,
    raw_query: &str,
) {
    if resolved.is_dir() {
        let readme = ["README.md", "index.md"]
            .iter()
            .map(|name| resolved.join(name))
            .find(|p| p.is_file());
        return match readme {
            Some(doc) => serve_markdown(request, slug, project, &doc),
            None => serve_dir_listing(request, slug, project, resolved),
        };
    }

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // `?raw=1` is the byte-passthrough behind every `<img>`/`<video>`/`<iframe>` src, the Copy
    // button's fetch, and (with `&dl=1`) the Download link — never wrapped in the page shell, so
    // it has to be checked before any of the view dispatch below.
    if query_flag(raw_query, "raw") {
        serve_raw_bytes(request, resolved, &ext, query_flag(raw_query, "dl"));
        return;
    }

    match ext.as_str() {
        "md" | "markdown" => serve_markdown(request, slug, project, resolved),
        _ if is_known_source_ext(&ext) || is_plain_text_ext(&ext) => {
            let Ok(bytes) = std::fs::read(resolved) else {
                let _ = request.respond(not_found());
                return;
            };
            serve_text(request, slug, project, resolved, &bytes, &ext);
        }
        _ => match known_binary_kind(&ext) {
            Some(kind) => serve_media(request, slug, project, resolved, kind),
            None => serve_unknown(request, slug, project, resolved),
        },
    }
}

fn serve_markdown(request: Request, slug: &str, project: &ProjectEntry, path: &Path) {
    let Ok(src) = std::fs::read_to_string(path) else {
        let _ = request.respond(not_found());
        return;
    };
    let (rendered, toc) = render_markdown(&src);
    let toc_html = render_toc(&toc);
    let content_html = file_view(&rendered, true, &toolbar_html(true, true));
    let active_rel = rel_path(&project.root, path);
    let nav_tree_html = render_nav_tree(&project.root, slug, &active_rel);
    let title = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let page = render_page(
        slug,
        &project.name,
        &nav_tree_html,
        &toc_html,
        title,
        &content_html,
    );
    respond_html(request, page);
}

/// A file already known (or sniffed) to be text: source code, a doc, or an unrecognised
/// extension that read back as valid UTF-8 with no NUL byte in it.
fn serve_text(
    request: Request,
    slug: &str,
    project: &ProjectEntry,
    path: &Path,
    bytes: &[u8],
    ext: &str,
) {
    let text = String::from_utf8_lossy(bytes);
    let lang = hljs_lang(ext);
    let inner = format!(
        "<pre><code class=\"language-{lang}\">{}</code></pre>",
        html_escape(&text)
    );
    let content_html = file_view(&inner, true, &toolbar_html(true, true));
    let active_rel = rel_path(&project.root, path);
    let nav_tree_html = render_nav_tree(&project.root, slug, &active_rel);
    let title = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let page = render_page(
        slug,
        &project.name,
        &nav_tree_html,
        "",
        title,
        &content_html,
    );
    respond_html(request, page);
}

/// A recognised image, video or PDF: embedded inline, in the app shell, via its own `?raw=1`
/// URL — nothing here reads the file itself, that's `serve_raw_bytes`'s job.
fn serve_media(request: Request, slug: &str, project: &ProjectEntry, path: &Path, kind: MediaKind) {
    let name = html_escape(path.file_name().and_then(|n| n.to_str()).unwrap_or(""));
    let media = match kind {
        MediaKind::Image => format!("<img src=\"?raw=1\" alt=\"{name}\">"),
        MediaKind::Video => "<video src=\"?raw=1\" controls></video>".to_string(),
        MediaKind::Pdf => format!("<iframe src=\"?raw=1\" title=\"{name}\"></iframe>"),
    };
    let inner = format!("<div class=\"file-media\">{media}</div>");
    let content_html = file_view(&inner, true, &toolbar_html(false, false));
    let active_rel = rel_path(&project.root, path);
    let nav_tree_html = render_nav_tree(&project.root, slug, &active_rel);
    let title = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let page = render_page(
        slug,
        &project.name,
        &nav_tree_html,
        "",
        title,
        &content_html,
    );
    respond_html(request, page);
}

/// An extension with no recognised meaning: too big to read at all, or read and not sniffed as
/// text. Either way, size and a Download link — a "Show text" button on the small-but-uncertain
/// case is the one thing the caller adds that this doesn't.
fn serve_unknown(request: Request, slug: &str, project: &ProjectEntry, path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        let _ = request.respond(not_found());
        return;
    };
    let size = metadata.len();
    if size > UNKNOWN_TEXT_SNIFF_LIMIT {
        serve_file_info(request, slug, project, path, size, false);
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        let _ = request.respond(not_found());
        return;
    };
    if looks_like_text(&bytes) {
        serve_text(request, slug, project, path, &bytes, "");
    } else {
        serve_file_info(request, slug, project, path, size, true);
    }
}

/// ponytail: "is this text" is a NUL-byte-and-UTF-8-validity guess, not a MIME sniff — good
/// enough for a local doc viewer deciding whether to auto-render; a real sniffer (the `infer` or
/// `content_inspector` crate) is the upgrade if this ever guesses wrong often enough to matter.
fn looks_like_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Size and a Download link, for a file this module won't render on its own — too big to read,
/// or read and not sniffed as text. `offer_show_text` is only true for the latter: a "Show text"
/// button that forces the client-side render, since the size ceiling already makes the fetch cheap.
fn serve_file_info(
    request: Request,
    slug: &str,
    project: &ProjectEntry,
    path: &Path,
    size: u64,
    offer_show_text: bool,
) {
    let name = html_escape(path.file_name().and_then(|n| n.to_str()).unwrap_or(""));
    let show_text_btn = if offer_show_text {
        "<button type=\"button\" class=\"file-btn\" id=\"file-show-text-btn\">Show text</button>"
    } else {
        ""
    };
    let inner = format!(
        "<div class=\"file-info\"><div class=\"file-info-name\">{name}</div><div class=\"file-info-size\">{}</div><div class=\"file-info-actions\"><a class=\"file-btn\" href=\"?raw=1&dl=1\">Download</a>{show_text_btn}</div></div>",
        human_size(size)
    );
    let content_html = file_view(&inner, false, "");
    let active_rel = rel_path(&project.root, path);
    let nav_tree_html = render_nav_tree(&project.root, slug, &active_rel);
    let title = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let page = render_page(
        slug,
        &project.name,
        &nav_tree_html,
        "",
        title,
        &content_html,
    );
    respond_html(request, page);
}

/// The byte passthrough behind `?raw=1` — the same bytes `serve_media`'s `<img>`/`<video>`/
/// `<iframe>` point at, the "Copy"/"Show text" buttons fetch, and (with `download`) the Download
/// link forces to disk rather than navigating to.
fn serve_raw_bytes(request: Request, path: &Path, ext: &str, download: bool) {
    let Ok(bytes) = std::fs::read(path) else {
        let _ = request.respond(not_found());
        return;
    };
    // A known-text extension gets a text content type here too — this is the same endpoint the
    // Copy button fetches for a markdown/source page, and a browser tab opened on it directly
    // should read as text, not download as an opaque blob.
    let mime = if is_doc_ext(ext) || is_known_source_ext(ext) || is_plain_text_ext(ext) {
        "text/plain; charset=utf-8"
    } else {
        mime_for_ext(ext)
    };
    let mut response = Response::from_data(bytes).with_header(content_type_header(mime));
    if download {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
            .replace('"', "'");
        response = response.with_header(
            Header::from_bytes(
                &b"Content-Disposition"[..],
                format!("attachment; filename=\"{name}\"").as_bytes(),
            )
            .expect("filename is a plain, quote-free string by the replace above"),
        );
    }
    let _ = request.respond(response);
}

fn serve_dir_listing(request: Request, slug: &str, project: &ProjectEntry, dir: &Path) {
    let mut entries: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();

    let active_rel = rel_path(&project.root, dir);
    let mut list_html = String::from("<ul>");
    for name in &entries {
        let href = if active_rel.is_empty() {
            format!("/{slug}/{name}")
        } else {
            format!("/{slug}/{active_rel}/{name}")
        };
        list_html.push_str(&format!(
            "<li><a href=\"{href}\">{}</a></li>",
            html_escape(name)
        ));
    }
    list_html.push_str("</ul>");
    if entries.is_empty() {
        list_html = "<p>This directory is empty.</p>".to_string();
    }

    let nav_tree_html = render_nav_tree(&project.root, slug, &active_rel);
    let title = if active_rel.is_empty() {
        &project.name
    } else {
        &active_rel
    };
    let page = render_page(slug, &project.name, &nav_tree_html, "", title, &list_html);
    respond_html(request, page);
}

fn serve_asset(request: Request, name: Option<&str>) {
    let asset = match name {
        Some("style.css") => Some(assets::style_css()),
        Some("script.js") => Some(assets::script_js()),
        _ => None,
    };
    let Some(asset) = asset else {
        let _ = request.respond(not_found());
        return;
    };
    let mut response = Response::from_data(asset.bytes.to_vec())
        .with_header(content_type_header(asset.content_type));
    if asset.gzip {
        response = response.with_header(
            Header::from_bytes(&b"Content-Encoding"[..], &b"gzip"[..])
                .expect("static header is valid"),
        );
    }
    let _ = request.respond(response);
}

// --- Content search (`/<slug>/_search?q=...`) ---

const MAX_SEARCH_FILES: usize = 200;
const MAX_LINES_PER_FILE: usize = 20;
const MAX_LINE_CHARS: usize = 300;

#[derive(serde::Serialize)]
struct SearchLineHit {
    line: usize,
    text: String,
}

#[derive(serde::Serialize)]
struct SearchFileHit {
    path: String,
    lines: Vec<SearchLineHit>,
}

#[derive(serde::Serialize)]
struct SearchResponse {
    query: String,
    results: Vec<SearchFileHit>,
    truncated: bool,
}

fn query_param(raw_query: &str, key: &str) -> String {
    raw_query
        .split('&')
        .find_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            (kv.next()? == key).then(|| percent_decode(&kv.next().unwrap_or("").replace('+', " ")))
        })
        .unwrap_or_default()
}

/// A bare boolean query flag — `?raw=1`, `?raw` and `?raw=` are all "present"; anything absent is
/// not. Good enough for the handful of one-off switches on the file routes; `query_param` is for
/// an actual value.
fn query_flag(raw_query: &str, key: &str) -> bool {
    raw_query
        .split('&')
        .any(|pair| pair.split('=').next() == Some(key))
}

/// ponytail: case-insensitive substring scan, no index, one file read per candidate. Fine for a
/// single project browsed locally; a real search index (tokenizing, ranking, incremental updates)
/// already exists in `crates/ubiq-host/src/search/` — this module deliberately doesn't depend on
/// `ubiq-host` (see the module doc comment), so copy from there if this ever needs to scale up.
fn serve_search(request: Request, project: &ProjectEntry, raw_query: &str) {
    let query = query_param(raw_query, "q").trim().to_string();
    if query.is_empty() {
        respond_json(
            request,
            SearchResponse {
                query,
                results: Vec::new(),
                truncated: false,
            },
        );
        return;
    }
    let needle = query.to_lowercase();

    let mut results = Vec::new();
    let mut truncated = false;
    for entry in ignore::WalkBuilder::new(&project.root)
        .build()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !is_doc_ext(&ext) && !is_known_source_ext(&ext) {
            continue;
        }
        if results.len() >= MAX_SEARCH_FILES {
            truncated = true;
            break;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut lines = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if !line.to_lowercase().contains(&needle) {
                continue;
            }
            let mut trimmed = line.trim().to_string();
            if trimmed.chars().count() > MAX_LINE_CHARS {
                trimmed = trimmed.chars().take(MAX_LINE_CHARS).collect::<String>();
                trimmed.push('\u{2026}');
            }
            lines.push(SearchLineHit {
                line: n + 1,
                text: trimmed,
            });
            if lines.len() >= MAX_LINES_PER_FILE {
                break;
            }
        }
        if !lines.is_empty() {
            results.push(SearchFileHit {
                path: rel_path(&project.root, path),
                lines,
            });
        }
    }

    respond_json(
        request,
        SearchResponse {
            query,
            results,
            truncated,
        },
    );
}

// --- Markdown rendering + TOC extraction ---

struct TocEntry {
    level: u8,
    id: String,
    text: String,
}

fn render_markdown(src: &str) -> (String, Vec<TocEntry>) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let events: Vec<Event> = Parser::new_ext(src, options).collect();

    let mut toc = Vec::new();
    let mut html_out = String::new();
    let mut slug_counts: HashMap<String, u32> = HashMap::new();
    let mut buf: Vec<Event> = Vec::new();
    let mut i = 0;

    while i < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[i] {
            let level = heading_level_num(*level);
            html::push_html(&mut html_out, buf.drain(..));

            i += 1;
            let mut inner = Vec::new();
            let mut text = String::new();
            while i < events.len() && !matches!(events[i], Event::End(TagEnd::Heading(_))) {
                if let Event::Text(t) | Event::Code(t) = &events[i] {
                    text.push_str(t);
                }
                inner.push(events[i].clone());
                i += 1;
            }
            i += 1; // consume End(Heading)

            let base = slugify(&text);
            let count = slug_counts.entry(base.clone()).or_insert(0);
            let id = if *count == 0 {
                base.clone()
            } else {
                format!("{base}-{count}")
            };
            *count += 1;

            let mut inner_html = String::new();
            html::push_html(&mut inner_html, inner.into_iter());
            html_out.push_str(&format!("<h{level} id=\"{id}\">{inner_html}</h{level}>\n"));
            toc.push(TocEntry { level, id, text });
        } else {
            buf.push(events[i].clone());
            i += 1;
        }
    }
    html::push_html(&mut html_out, buf.drain(..));
    (html_out, toc)
}

fn heading_level_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in text.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

fn render_toc(toc: &[TocEntry]) -> String {
    toc.iter()
        .map(|entry| {
            format!(
                "<li class=\"toc-item level-{}\"><a href=\"#{}\" class=\"toc-link\">{}</a></li>",
                entry.level,
                entry.id,
                html_escape(&entry.text)
            )
        })
        .collect()
}

// --- Nav tree (walks the project respecting .gitignore, via the `ignore` crate) ---

#[derive(Default)]
struct TrieNode {
    children: BTreeMap<String, TrieNode>,
    is_file: bool,
}

fn render_nav_tree(root: &Path, slug: &str, active_rel: &str) -> String {
    let mut trie = TrieNode::default();
    for entry in ignore::WalkBuilder::new(root)
        .build()
        .filter_map(|e| e.ok())
    {
        if entry.depth() == 0 {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        insert_into_trie(&mut trie, rel, is_file);
    }
    let mut out = String::from("<ul class=\"nav-tree\">");
    render_trie(&trie, "", slug, active_rel, &mut out);
    out.push_str("</ul>");
    out
}

fn insert_into_trie(root: &mut TrieNode, rel: &Path, is_file: bool) {
    let names: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut node = root;
    let last = names.len().saturating_sub(1);
    for (i, name) in names.into_iter().enumerate() {
        node = node.children.entry(name).or_default();
        if i == last {
            node.is_file = is_file;
        }
    }
}

const NAV_CHEVRON_SVG: &str = r#"<svg class="nav-chevron" viewBox="0 0 16 16" fill="currentColor"><path d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06z"></path></svg>"#;
const NAV_DIR_ICON_SVG: &str = r#"<svg class="nav-dir-icon" viewBox="0 0 16 16" fill="currentColor"><path d="M1.75 1A1.75 1.75 0 0 0 0 2.75v10.5C0 14.216.784 15 1.75 15h12.5A1.75 1.75 0 0 0 16 13.25v-8.5A1.75 1.75 0 0 0 14.25 3H7.5a.25.25 0 0 1-.2-.1l-.9-1.2C6.07 1.26 5.55 1 5 1H1.75z"></path></svg>"#;
const NAV_DOC_ICON_SVG: &str = r#"<svg class="nav-doc-icon" viewBox="0 0 16 16" fill="currentColor"><path d="M0 1.75C0 .784.784 0 1.75 0h8.5C10.716 0 11 .284 11 .75v3.5a.75.75 0 0 1 0 1.5h-1.5a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h7.5a.25.25 0 0 0 .25-.25v-1.5a.75.75 0 0 1 1.5 0v1.5A1.75 1.75 0 0 1 9.25 16h-7.5A1.75 1.75 0 0 1 0 14.25v-7.5z"></path><path d="M5 1.75C5 .784 5.784 0 6.75 0h7.5C15.216 0 16 .784 16 1.75v7.5A1.75 1.75 0 0 1 14.25 11h-7.5A1.75 1.75 0 0 1 5 9.25v-7.5zm1.75-.25a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h7.5a.25.25 0 0 0 .25-.25v-7.5a.25.25 0 0 0-.25-.25h-7.5z"></path></svg>"#;

fn render_trie(node: &TrieNode, prefix: &str, slug: &str, active_rel: &str, out: &mut String) {
    for (name, child) in &node.children {
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let lower = html_escape(&name.to_lowercase());
        if child.is_file {
            let active = if rel == active_rel { " active" } else { "" };
            // The "docs only" sidebar toggle filters on this in the browser — same md/markdown
            // rule `serve_path` uses to pick `serve_markdown` over `serve_source`/`serve_raw`.
            let doc_ext = Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| is_doc_ext(&e.to_ascii_lowercase()))
                .unwrap_or(false);
            let doc = if doc_ext { "1" } else { "0" };
            out.push_str(&format!(
                "<li class=\"nav-item\" data-title=\"{lower}\" data-filename=\"{lower}\" data-doc=\"{doc}\"><a href=\"/{slug}/{rel}\" class=\"nav-link{active}\">{NAV_DOC_ICON_SVG}<span class=\"nav-title nav-label-title\">{name}</span><span class=\"nav-title nav-label-filename\">{name}</span></a></li>",
                name = html_escape(name)
            ));
        } else {
            out.push_str(&format!(
                "<li class=\"nav-item nav-dir\" data-title=\"{lower}\" data-filename=\"{lower}\"><div class=\"nav-dir-header\">{NAV_CHEVRON_SVG}{NAV_DIR_ICON_SVG}<span class=\"nav-dir-title nav-label-title\">{name}</span><span class=\"nav-dir-title nav-label-filename\">{name}</span></div><ul class=\"nav-subgroup\">",
                name = html_escape(name)
            ));
            render_trie(child, &rel, slug, active_rel, out);
            out.push_str("</ul></li>");
        }
    }
}

// --- The file-view chrome: the toolbar and the wide wrapper every file page (but not a
// directory listing) is wrapped in ---

/// `wide` is a class, not a layout decision made here — `style.css` is what actually widens
/// `.content-wrapper` when a `.file-view.wide` shows up inside it, via `:has()`, so this stays
/// free of anything about *how* wide.
fn file_view(inner_html: &str, wide: bool, toolbar: &str) -> String {
    let wide_class = if wide { " wide" } else { "" };
    format!(
        "<div class=\"file-view{wide_class}\">{toolbar}<div id=\"file-content\" class=\"file-content\">{inner_html}</div></div>"
    )
}

/// Copy and Find both act on `#file-content` client-side, so both are meaningless over an image
/// or a video — `serve_media` is the one caller that turns both off. Download is universal: even
/// a file this module refuses to render is still one the browser can save.
///
/// The find bar's markup ships here too, hidden, rather than being built by `script.js` — one
/// less thing the client has to construct, and it means every file view carries the same DOM
/// whether JS runs immediately or after an SPA swap re-injects this exact HTML.
fn toolbar_html(copy: bool, find: bool) -> String {
    let mut html = String::from("<div class=\"file-toolbar\">");
    if copy {
        html.push_str(
            "<button type=\"button\" class=\"file-btn\" id=\"file-copy-btn\">Copy</button>",
        );
    }
    html.push_str("<a class=\"file-btn\" href=\"?raw=1&dl=1\">Download</a>");
    if find {
        html.push_str(
            "<button type=\"button\" class=\"file-btn\" id=\"file-find-btn\">Find</button>",
        );
    }
    html.push_str("</div>");
    if find {
        html.push_str(
            "<div class=\"file-find-bar\" id=\"file-find-bar\" hidden>\
               <input type=\"text\" id=\"file-find-input\" placeholder=\"Find in file\u{2026}\" autocomplete=\"off\">\
               <span class=\"file-find-count\" id=\"file-find-count\"></span>\
               <button type=\"button\" class=\"file-find-nav\" id=\"file-find-prev\" aria-label=\"Previous match\">\u{2191}</button>\
               <button type=\"button\" class=\"file-find-nav\" id=\"file-find-next\" aria-label=\"Next match\">\u{2193}</button>\
               <button type=\"button\" class=\"file-find-nav\" id=\"file-find-close\" aria-label=\"Close find\">\u{2715}</button>\
             </div>",
        );
    }
    html
}

// --- Template substitution ---

fn render_page(
    slug: &str,
    project_name: &str,
    nav_tree_html: &str,
    toc_html: &str,
    title: &str,
    content_html: &str,
) -> String {
    let toc_block = if toc_html.is_empty() {
        String::new()
    } else {
        format!(
            "<aside class=\"app-toc\"><div class=\"toc-header\">On this page</div><ul class=\"toc-list\">{toc_html}</ul></aside>"
        )
    };
    assets::TEMPLATE_HTML
        .replace("{{PROJECT_NAME}}", &html_escape(project_name))
        .replace("{{TITLE}}", &html_escape(title))
        .replace("{{HOME_URL}}", &format!("/{slug}/"))
        .replace("{{NAV_TREE}}", nav_tree_html)
        .replace("{{CONTENT}}", content_html)
        .replace("{{TOC}}", &toc_block)
        .replace("{{VERSION_SHORT}}", &html_escape(&crate::version::short()))
        .replace("{{VERSION_FULL}}", &html_escape(crate::version::FULL))
}

// --- Small helpers: escaping, extension maps, HTTP plumbing ---

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_doc_ext(ext: &str) -> bool {
    matches!(ext, "md" | "markdown")
}

/// Obviously text, but not source code and not covered by `is_known_source_ext`'s syntax-aware
/// list — these still get the plain code-viewer, just with no highlight language.
fn is_plain_text_ext(ext: &str) -> bool {
    matches!(ext, "txt" | "log" | "csv")
}

fn is_known_source_ext(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "rb"
            | "java"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "sh"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "css"
            | "html"
            | "sql"
    )
}

fn hljs_lang(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "cpp" | "hpp" => "cpp",
        "cs" => "csharp",
        "sh" => "bash",
        "yml" => "yaml",
        other => other,
    }
}

/// The extensions `serve_media` knows how to embed. Fonts and archives are deliberately not
/// here even though `mime_for_ext` names them — there is nothing to embed a `.zip` as, so those
/// fall through to `serve_unknown`'s size-and-sniff treatment like any other extension it has
/// never heard of, and end up in the same info-panel-plus-Download view.
#[derive(Clone, Copy)]
enum MediaKind {
    Image,
    Video,
    Pdf,
}

fn known_binary_kind(ext: &str) -> Option<MediaKind> {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" => Some(MediaKind::Image),
        "mp4" => Some(MediaKind::Video),
        "pdf" => Some(MediaKind::Pdf),
        _ => None,
    }
}

/// `1.2 MB`, `340 KB`, `18 B` — one decimal past bytes, matching how every OS file browser
/// already renders a size, so the info panel needs no legend to explain it.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" => "text/plain; charset=utf-8",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "mp4" => "video/mp4",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn content_type_header(mime: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], mime.as_bytes()).expect("static header is valid")
}

fn respond_html(request: Request, body: String) {
    let response =
        Response::from_string(body).with_header(content_type_header("text/html; charset=utf-8"));
    let _ = request.respond(response);
}

fn respond_json(request: Request, body: SearchResponse) {
    let json = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
    let response = Response::from_string(json)
        .with_header(content_type_header("application/json; charset=utf-8"));
    let _ = request.respond(response);
}

fn not_found() -> Response<Cursor<Vec<u8>>> {
    Response::from_string("404 Not Found").with_status_code(404)
}
