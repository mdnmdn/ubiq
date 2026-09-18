//! The `ubiq-help` server: Ubiq's own documentation, as tools a hosted agent can call.
//!
//! An agent that is asked how Ubiq itself works has no better source than the manual behind the
//! `?` in the titlebar — the same pages, the same catalogue, read here instead of guessed at.
//! **A page is addressed by its `id`, never a path**, exactly as [`super::kb`] addresses a
//! document by a source name rather than a filesystem path: the id is what
//! [`crate::help::Help::ready`]'s catalogue carries, what the panel's own links use, and what
//! survives a page moving between folders.
//!
//! **No tool here ever answers an error.** A build that ships no documentation is a downgrade,
//! not a fault — [`crate::help`]'s own module doc says why — and every tool below answers that
//! case as data: `list_help_pages` comes back with an empty list and a `note` explaining why,
//! and the other three answer the same shape they would for a page or a search with nothing
//! found. A model reading these tools should never have to distinguish "nothing matched" from
//! "this build has no manual" through an `isError`.
//!
//! **Weighting, for `search_help`.** A hit in `keywords`, `title` or `summary` is what a page's
//! author put there *for* search — the words a reader types that the prose never uses — so it
//! ranks above a hit buried in the body.

use serde_json::{Value, json};
use ubiq_proto::help::{HelpCatalog, HelpPage};

use super::HelpReach;

/// Search hits in the frontmatter fields outrank a body hit by this much, so a page whose author
/// named the right keyword surfaces over one that merely mentions the word once in passing.
const FRONTMATTER_WEIGHT: u32 = 100;

pub fn call(tool: &str, arguments: &Value, reach: &HelpReach) -> Result<Value, String> {
    match tool {
        "list_help_pages" => Ok(list_pages(reach)),
        "read_help_page" => read_page(arguments, reach),
        "search_help" => search(arguments, reach),
        "help_for_context" => Ok(help_for_context(arguments, reach)),
        _ => Err(format!(
            "unknown tool: {}/{tool}",
            super::catalogue::UBIQ_HELP
        )),
    }
}

/// Every page in nav order, or an empty list with a note when this build has none.
fn list_pages(reach: &HelpReach) -> Value {
    let Some((_, catalog)) = reach.help.ready() else {
        return json!({"pages": [], "note": unavailable_note(reach)});
    };
    json!({"pages": nav_order(&catalog).map(page_summary).collect::<Vec<_>>()})
}

/// One page's own body: its markdown with the frontmatter the packer already stripped — the
/// bundle carries body text only, on [`crate::help`]'s own contract with the packer.
fn read_page(arguments: &Value, reach: &HelpReach) -> Result<Value, String> {
    let id = required_str(arguments, "id")?;
    let Some((root, catalog)) = reach.help.ready() else {
        return Ok(json!({"found": false, "note": unavailable_note(reach)}));
    };
    let Some(page) = catalog.page(id) else {
        return Ok(json!({
            "found": false,
            "note": format!("'{id}' is not a page of this manual"),
        }));
    };
    let path = std::path::Path::new(&root).join(&page.path);
    let body = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", page.path))?;
    Ok(json!({
        "found": true,
        "id": page.id,
        "title": page.title,
        "body": body,
    }))
}

/// Matches over the frontmatter fields and the body, each hit as the line it landed on.
fn search(arguments: &Value, reach: &HelpReach) -> Result<Value, String> {
    let query = required_str(arguments, "query")?;
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Err("search_help needs a non-empty 'query' argument".to_string());
    }

    let Some((root, catalog)) = reach.help.ready() else {
        return Ok(json!({"results": [], "note": unavailable_note(reach)}));
    };

    let mut hits: Vec<(u32, Value)> = Vec::new();
    for page in nav_order(&catalog) {
        let mut score = 0u32;
        let mut lines: Vec<String> = Vec::new();

        if page.title.to_lowercase().contains(&needle) {
            score += FRONTMATTER_WEIGHT;
            lines.push(page.title.clone());
        }
        if page.summary.to_lowercase().contains(&needle) {
            score += FRONTMATTER_WEIGHT;
            lines.push(page.summary.clone());
        }
        if page
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(&needle))
        {
            score += FRONTMATTER_WEIGHT;
        }

        let path = std::path::Path::new(&root).join(&page.path);
        if let Ok(body) = std::fs::read_to_string(&path) {
            for line in body.lines() {
                if line.to_lowercase().contains(&needle) {
                    score += 1;
                    lines.push(line.trim().to_string());
                }
            }
        }

        if score > 0 {
            hits.push((
                score,
                json!({
                    "id": page.id,
                    "title": page.title,
                    "summary": page.summary,
                    "lines": lines,
                }),
            ));
        }
    }
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));

    Ok(json!({
        "results": hits.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
    }))
}

/// The page bound to a context key, falling through exactly as the panel's own rungs do (§5 of
/// the design), ending at `index` if the key names nothing — so this always finds something when
/// the manual has an index page at all.
fn help_for_context(arguments: &Value, reach: &HelpReach) -> Value {
    let Some(key) = arguments.get("key").and_then(Value::as_str) else {
        return json!({"found": false, "note": "help_for_context needs a 'key' argument"});
    };
    let Some((_, catalog)) = reach.help.ready() else {
        return json!({"found": false, "note": unavailable_note(reach)});
    };
    let page = catalog.for_context(key).or_else(|| catalog.page("index"));
    match page {
        Some(page) => json!({"found": true, "id": page.id, "title": page.title}),
        None => json!({"found": false, "note": format!("no page is bound to '{key}'")}),
    }
}

/// The pages in the order the tree draws them, `nav` first — a page the nav does not list (a
/// leftover id, or one authored but not yet wired into the tree) is still answered, appended
/// after, so nothing a real bundle carries is ever silently dropped from these tools.
fn nav_order(catalog: &HelpCatalog) -> impl Iterator<Item = &HelpPage> {
    let ordered = catalog
        .nav
        .iter()
        .filter_map(|id| catalog.page(id))
        .collect::<Vec<_>>();
    let rest = catalog
        .pages
        .iter()
        .filter(move |page| !catalog.nav.iter().any(|id| id == &page.id));
    ordered.into_iter().chain(rest)
}

fn page_summary(page: &HelpPage) -> Value {
    json!({"id": page.id, "title": page.title, "summary": page.summary})
}

fn unavailable_note(reach: &HelpReach) -> String {
    reach
        .help
        .unavailable_reason()
        .unwrap_or_else(|| "no help content is available".to_string())
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    match arguments.get(key) {
        Some(Value::String(value)) => Ok(value.as_str()),
        None | Some(Value::Null) => Err(format!("{key} is required")),
        Some(_) => Err(format!("{key} must be a string")),
    }
}
