//! Brand marks (`renderers/shared/brand-marks.mjs`, `00` §5.3, `03` §4.5): the 107 built-in vector
//! marks as embedded data (`data/brand-marks.json`, dumped by `gen-brand-marks.mjs`), the
//! `brand` lookup, the 16x16 badge a node wears and V8, the brand-preparation errors.
//!
//! A node's `brand` is a catalogue id, title, alias or domain (text), or `{url, sha256}`, a
//! digest-pinned capture of a site icon. Text resolves like `findBrandMark`: the raw form, the
//! dashed form and the compact form of each of `id`, `title` and `aliases`, the first mark to claim
//! a form winning, or the longest domain that equals the URL's host or is a parent of it.
//!
//! Not ported (D19): the capture itself. A pinned `{url, sha256}` cannot be verified or drawn
//! without the network and the image bytes, so it is accepted, drawn as the globe fallback and
//! reported by a non-blocking `brand/capture-unavailable` warning (Archify fails closed and draws
//! the captured image; see D37).

use std::collections::HashMap;
use std::sync::OnceLock;

use kurbo::{BezPath, Circle, RoundedRect, Shape as _};
use serde::Deserialize;
use serde_json::Map;

use crate::diag::{Diagnostic, Subject};
use crate::model::Doc;
use crate::model::common::BrandMark;
use crate::scene::{Bounds, Cmd, Fill, GroupId, Layer, PathShape, RectShape, SceneBuilder, Shape, Stroke};
use crate::tokens::Token;

const RAW: &str = include_str!("data/brand-marks.json");

/// The badge: 16x16, plate radius 4, the vector inset by 3 on every side.
pub const SIZE: f64 = 16.0;
pub const RADIUS: f64 = 4.0;
pub const INSET: f64 = 3.0;
/// The badge sits `22` from the node's right edge and `6` below its top (`x + width - 22`).
pub const RIGHT: f64 = 22.0;
pub const TOP: f64 = 6.0;
/// `.brand-mark-frame` stroke, screen px.
pub const FRAME_STROKE: f64 = 0.8;
/// `.brand-mark-fallback` stroke (1.15) in the 0.8-scaled group's user space.
const GLOBE_STROKE: f64 = 1.15;
const GLOBE_SCALE: f64 = SIZE / 20.0;

/// One built-in mark.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Mark {
    pub id: String,
    pub title: String,
    pub category: String,
    pub aliases: Vec<String>,
    pub domains: Vec<String>,
    pub view_box: f64,
    /// `RRGGBB`.
    pub hex: String,
    pub path: String,
    pub source: String,
}

struct Tables {
    marks: Vec<Mark>,
    by_lookup: HashMap<String, usize>,
    by_domain: Vec<(String, usize)>,
}

fn tables() -> &'static Tables {
    static CELL: OnceLock<Tables> = OnceLock::new();
    CELL.get_or_init(|| {
        let marks: Vec<Mark> = serde_json::from_str(RAW).expect("data/brand-marks.json");
        let mut by_lookup = HashMap::new();
        let mut by_domain: Vec<(String, usize)> = Vec::new();
        for (i, mark) in marks.iter().enumerate() {
            for value in std::iter::once(&mark.id).chain(std::iter::once(&mark.title)).chain(&mark.aliases) {
                for form in lookup_forms(value) {
                    by_lookup.entry(form).or_insert(i);
                }
            }
            for domain in &mark.domains {
                // `Map.set`: a later mark claims the domain, at the first one's position.
                match by_domain.iter_mut().find(|(d, _)| d == domain) {
                    Some(slot) => slot.1 = i,
                    None => by_domain.push((domain.clone(), i)),
                }
            }
        }
        Tables { marks, by_lookup, by_domain }
    })
}

/// Every built-in mark, in catalogue order.
pub fn marks() -> &'static [Mark] {
    &tables().marks
}

/// `lookupForms`: lower-cased and trimmed, then `[\s_]+` as `-`, then `[\s_.-]+` removed.
fn lookup_forms(value: &str) -> Vec<String> {
    let raw = value.trim().to_lowercase();
    if raw.is_empty() {
        return Vec::new();
    }
    let dashed = squash(&raw, |c| c.is_whitespace() || c == '_', Some('-'));
    let compact = squash(&raw, |c| c.is_whitespace() || matches!(c, '_' | '.' | '-'), None);
    let mut out = vec![raw];
    for form in [dashed, compact] {
        if !out.contains(&form) {
            out.push(form);
        }
    }
    out
}

/// Replaces each run of characters matching `is_sep` by `with` (or drops it).
fn squash(s: &str, is_sep: impl Fn(char) -> bool, with: Option<char>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_run = false;
    for c in s.chars() {
        if is_sep(c) {
            if !in_run {
                out.extend(with);
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

/// A parsed `http(s)` URL: the lower-cased host and the normalised `href`.
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub host: String,
    pub href: String,
}

/// `asUrl`: `new URL(value)` for an `http:` or `https:` URL (the `scheme://` form; no
/// percent-decoding or IDNA).
pub fn parse_url(value: &str) -> Option<Url> {
    let s = value.trim_matches(|c: char| c <= ' ');
    let lower = s.to_ascii_lowercase();
    let (scheme, rest) = if lower.starts_with("https://") {
        ("https", &s[8..])
    } else if lower.starts_with("http://") {
        ("http", &s[7..])
    } else {
        return None;
    };
    let end = rest.find(['/', '?', '#', '\\']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (host, port) = match hostport.rfind(':') {
        Some(i) if !hostport.ends_with(']') => (&hostport[..i], Some(&hostport[i + 1..])),
        _ => (hostport, None),
    };
    let host = host.to_ascii_lowercase();
    let bad_host = host.is_empty()
        || host.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '^' | '|' | '%' | '\\' | '#' | '/' | '?'))
        || (host.contains(['[', ']']) && !(host.starts_with('[') && host.ends_with(']')));
    let bad_port = port.is_some_and(|p| !p.chars().all(|c| c.is_ascii_digit()) || p.parse::<u32>().is_ok_and(|n| n > 65535));
    if bad_host || bad_port {
        return None;
    }
    let default_port = if scheme == "https" { "443" } else { "80" };
    let port = port.filter(|p| !p.is_empty() && *p != default_port);
    let tail = tail.replace('\\', "/");
    let tail = if tail.starts_with('/') { tail } else { format!("/{tail}") };
    let href = format!(
        "{scheme}://{}{host}{}{tail}",
        userinfo.map(|u| format!("{u}@")).unwrap_or_default(),
        port.map(|p| format!(":{p}")).unwrap_or_default(),
    );
    Some(Url { host, href })
}

/// `domainMark`: the mark whose domain equals the host or is a parent of it, the longest first.
fn domain_mark(host: &str) -> Option<&'static Mark> {
    let t = tables();
    let host = host.to_lowercase();
    let host = host.trim_end_matches('.');
    t.by_domain
        .iter()
        .filter(|(domain, _)| host == domain || host.ends_with(&format!(".{domain}")))
        .max_by_key(|(domain, _)| domain.len())
        .map(|&(_, i)| &t.marks[i])
}

/// `findBrandMark`: a URL by its domain, anything else by id, title or alias.
pub fn find(value: &str) -> Option<&'static Mark> {
    if let Some(url) = parse_url(value) {
        return domain_mark(&url.host);
    }
    let t = tables();
    lookup_forms(value).iter().find_map(|form| t.by_lookup.get(form)).map(|&i| &t.marks[i])
}

/// What a node's `brand` resolves to.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// A built-in mark.
    Preset(&'static Mark),
    /// A digest-pinned capture: drawn as the globe (not captured here).
    Pinned { url: String },
}

/// `brandMarkFor`, once V8 passed. `None` for a text that names no mark.
pub fn resolve(brand: &BrandMark) -> Option<Resolved> {
    match brand {
        BrandMark::Text(text) => find(text).map(Resolved::Preset),
        BrandMark::Pinned(p) => Some(Resolved::Pinned { url: p.url.clone() }),
    }
}

/// `brandMetadataFor`: the `data-node-brand*` attributes of a node group.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub brand: String,
    pub id: String,
    pub status: &'static str,
    pub source: String,
}

pub fn meta(brand: &BrandMark) -> Option<Meta> {
    Some(match resolve(brand)? {
        Resolved::Preset(m) => Meta { brand: m.title.clone(), id: m.id.clone(), status: "preset", source: m.source.clone() },
        Resolved::Pinned { url } => {
            let host = parse_url(&url).map(|u| u.host).unwrap_or_else(|| url.clone());
            Meta { brand: host.clone(), id: host, status: "captured", source: url }
        }
    })
}

fn cmds(path: &BezPath) -> Vec<Cmd> {
    use kurbo::PathEl;
    let pt = |p: kurbo::Point| [p.x, p.y];
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) => Cmd::M(pt(p)),
            PathEl::LineTo(p) => Cmd::L(pt(p)),
            PathEl::QuadTo(c, e) => Cmd::Q(pt(c), pt(e)),
            PathEl::CurveTo(c1, c2, e) => Cmd::C(pt(c1), pt(c2), pt(e)),
            PathEl::ClosePath => Cmd::Z,
        })
        .collect()
}

fn hex_value(hex: &str) -> u32 {
    u32::from_str_radix(hex, 16).unwrap_or(0)
}

/// Pushes a node's badge into its group, after the sigil: `renderBrandMark(node, {x: right - 22,
/// y: top + 6})`. Nothing for a node without a `brand`.
pub fn push_badge(b: &mut SceneBuilder, group: GroupId, brand: Option<&BrandMark>, right: f64, top: f64) {
    let Some(brand) = brand else { return };
    for shape in badge(brand, right - RIGHT, top + TOP) {
        b.push_in(group, Layer::Nodes, shape);
    }
}

/// The badge frame outline in its own 16x16 space (`.brand-mark-frame`, `rx` = `radius`).
pub fn frame_cmds(radius: f64) -> Vec<Cmd> {
    cmds(&RoundedRect::new(0.0, 0.0, SIZE, SIZE, radius).to_path(0.01))
}

/// `renderBrandMark(node, {x, y})`: the badge's shapes, in paint order (plate, vector, frame), at
/// the badge's top-left `(x, y)`. Empty when the text names no mark.
pub fn badge(brand: &BrandMark, x: f64, y: f64) -> Vec<Shape> {
    let Some(resolved) = resolve(brand) else { return Vec::new() };
    let plate = Shape::Rect(RectShape {
        rect: Bounds::new(x, y, SIZE, SIZE),
        radius: RADIUS,
        fill: Some(Fill::new(Token::BrandBadge)),
        stroke: None,
    });
    let frame = Shape::Path(PathShape {
        cmds: frame_cmds(RADIUS),
        transform: [1.0, 0.0, 0.0, 1.0, x, y],
        stroke: Some(Stroke::solid(Token::BrandFrame, FRAME_STROKE)),
        fill: None,
        opacity: 1.0,
        non_scaling: true,
    });
    let mut out = vec![plate];
    match resolved {
        Resolved::Preset(mark) => {
            let s = (SIZE - INSET * 2.0) / mark.view_box;
            out.push(Shape::Path(PathShape {
                cmds: cmds(&BezPath::from_svg(&mark.path).unwrap_or_default()),
                transform: [s, 0.0, 0.0, s, x + INSET, y + INSET],
                stroke: None,
                fill: Some(Fill::new(Token::BrandInk(hex_value(&mark.hex)))),
                opacity: 1.0,
                non_scaling: false,
            }));
        }
        Resolved::Pinned { .. } => out.extend(globe(x, y)),
    }
    out.push(frame);
    out
}

/// `.brand-mark-fallback`: a circle and two meridians on a 20 grid, scaled by `16 / 20`.
fn globe(x: f64, y: f64) -> Vec<Shape> {
    const MERIDIANS: &str = "M4.8 10h10.4M10 4.8c1.6 1.6 2.4 3.3 2.4 5.2s-.8 3.6-2.4 5.2M10 4.8C8.4 6.4 7.6 8.1 7.6 10s.8 3.6 2.4 5.2";
    let stroke = || Some(Stroke::solid(Token::BrandGlyph, GLOBE_STROKE * GLOBE_SCALE));
    let part = |cmds: Vec<Cmd>| {
        Shape::Path(PathShape {
            cmds,
            transform: [GLOBE_SCALE, 0.0, 0.0, GLOBE_SCALE, x, y],
            stroke: stroke(),
            fill: None,
            opacity: 1.0,
            non_scaling: false,
        })
    };
    vec![part(cmds(&Circle::new((10.0, 10.0), 5.2).to_path(0.01))), part(cmds(&BezPath::from_svg(MERIDIANS).unwrap_or_default()))]
}

// ---- V8 ---------------------------------------------------------------------------------------

/// The authored brands of a document: the collection's name and `(index, brand)`.
fn brands(doc: &Doc) -> (&'static str, Vec<(usize, &BrandMark)>) {
    fn of<'a, T: 'a>(items: &'a [T], get: impl Fn(&'a T) -> Option<&'a BrandMark>) -> Vec<(usize, &'a BrandMark)> {
        items.iter().enumerate().filter_map(|(i, n)| get(n).map(|b| (i, b))).collect()
    }
    match doc {
        Doc::Architecture(d) => ("components", of(&d.components, |n| n.brand.as_ref())),
        Doc::Workflow(d) => ("nodes", of(&d.nodes, |n| n.brand.as_ref())),
        Doc::Sequence(d) => ("participants", of(&d.participants, |n| n.brand.as_ref())),
        Doc::Dataflow(d) => ("nodes", of(&d.nodes, |n| n.brand.as_ref())),
        Doc::Lifecycle(d) => ("states", of(&d.states, |n| n.brand.as_ref())),
    }
}

/// `suggestions`: the five ids closest to `value`, those that contain it (or are contained by it)
/// first, then by id.
fn suggestions(value: &str) -> Vec<&'static str> {
    let needle = lookup_forms(value).into_iter().next().unwrap_or_default();
    let mut scored: Vec<(u8, &'static str)> = marks()
        .iter()
        .map(|m| {
            let near = lookup_forms(&m.id).iter().any(|form| form.contains(&needle) || needle.contains(form.as_str()));
            (u8::from(!near), m.id.as_str())
        })
        .collect();
    scored.sort();
    scored.into_iter().take(5).map(|(_, id)| id).collect()
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The receipt's `error` for failed brand preparation.
pub fn error_text(diagnostics: &[Diagnostic]) -> String {
    let lines: Vec<&str> = diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    format!("Brand mark validation failed:\n- {}", lines.join("\n- "))
}

/// V8 (`prepareDiagramBrandMarks`): `brand/unknown` for a text that is no built-in mark,
/// `brand/unpinned-url` for an `http(s)` URL that is not a built-in site, in document order, and a
/// `brand/capture-unavailable` **warning** for each pinned `{url, sha256}` (not verified, drawn as
/// the globe).
pub fn check(doc: &Doc) -> Vec<Diagnostic> {
    let diagram_type = doc.diagram_type();
    let (collection, list) = brands(doc);
    let subject = || {
        let mut s = Subject::of(diagram_type);
        s.collection = Some(collection.to_owned());
        s
    };
    let mut out = Vec::new();
    for (index, brand) in list {
        match brand {
            BrandMark::Pinned(p) => out.push(
                Diagnostic::warning(
                    "brand/capture-unavailable",
                    &format!(
                        "/{collection}/{index}/brand could not reproduce the pinned capture: this build does not fetch brand images ({}); the globe mark is drawn.",
                        p.url
                    ),
                )
                .with_subject(subject())
                .with_evidence(Map::new())
                .with_fixes(["choose an ID from `archify brands`"]),
            ),
            BrandMark::Text(text) if find(text).is_some() => {}
            BrandMark::Text(text) => {
                if let Some(url) = parse_url(text) {
                    out.push(
                        Diagnostic::error(
                            "brand/unpinned-url",
                            &format!(
                                "/{collection}/{index}/brand {} is an unpinned URL; capture it first with `archify brands capture {} --json`",
                                json_string(text),
                                url.href
                            ),
                        )
                        .with_subject(subject())
                        .with_evidence(Map::new())
                        .with_fixes(["run `archify brands capture <url> --json` and author the returned digest-pinned brand object"]),
                    );
                } else {
                    out.push(
                        Diagnostic::error(
                            "brand/unknown",
                            &format!(
                                "/{collection}/{index}/brand {} is not a built-in brand; closest IDs: {}",
                                json_string(text),
                                suggestions(text).join(", ")
                            ),
                        )
                        .with_subject(subject())
                        .with_evidence(Map::new())
                        .with_fixes([
                            "choose an ID from `archify brands`",
                            "run `archify brands capture <url> --json` for an unknown official site",
                        ]),
                    );
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> BrandMark {
        BrandMark::Text(s.to_owned())
    }

    #[test]
    fn the_catalogue_is_107_marks_with_unique_ids() {
        assert_eq!(marks().len(), 107);
        let mut ids: Vec<&str> = marks().iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 107);
        assert!(ids.iter().all(|id| id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')));
    }

    #[test]
    fn lookup_takes_ids_titles_aliases_and_domains() {
        assert_eq!(find("claude").unwrap().id, "claude");
        assert_eq!(find("Claude AI").unwrap().id, "claude");
        assert_eq!(find("ChatGPT").unwrap().id, "openai");
        assert_eq!(find("https://claude.ai/chat").unwrap().id, "claude");
        assert_eq!(find("https://app.claude.ai").unwrap().id, "claude");
        assert!(find("https://example.com/x").is_none());
        assert!(find("nope-brand").is_none());
        assert_eq!(find("google gemini").unwrap().id, "google-gemini");
    }

    #[test]
    fn urls_parse_like_the_url_class() {
        let u = parse_url("HTTPS://Example.COM:443/a?b#c").unwrap();
        assert_eq!((u.host.as_str(), u.href.as_str()), ("example.com", "https://example.com/a?b#c"));
        assert_eq!(parse_url("http://example.com").unwrap().href, "http://example.com/");
        assert!(parse_url("ftp://example.com").is_none());
        assert!(parse_url("https://").is_none());
        assert!(parse_url("claude").is_none());
    }

    #[test]
    fn the_badge_is_plate_vector_frame() {
        let shapes = badge(&text("claude"), 100.0, 50.0);
        assert_eq!(shapes.len(), 3);
        let Shape::Path(p) = &shapes[1] else { panic!("vector") };
        let s = 10.0 / find("claude").unwrap().view_box;
        assert_eq!(p.transform, [s, 0.0, 0.0, s, 103.0, 53.0]);
        assert!(matches!(p.fill, Some(Fill { token: Token::BrandInk(_), .. })));
        assert!(badge(&text("nope"), 0.0, 0.0).is_empty());
        let pinned = BrandMark::Pinned(crate::model::common::PinnedBrand { url: "https://example.com/".into(), sha256: "a".repeat(64) });
        assert_eq!(badge(&pinned, 0.0, 0.0).len(), 4);
    }

    #[test]
    fn suggestions_follow_the_closest_ids() {
        assert_eq!(suggestions("Clod AI"), ["airtable", "alibaba-cloud", "angular", "ansible", "anthropic"]);
        assert_eq!(suggestions("gemin")[0], "google-gemini");
    }
}
