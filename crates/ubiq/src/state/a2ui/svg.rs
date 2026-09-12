//! Agent-authored SVG, read strictly enough to hand to a rasteriser.
//!
//! **This is a security boundary, not a tidy-up.** The markup arrives from a language model and
//! ends up in gpui's rasteriser, which resolves what it is given: a `<use>`, an `<image>`, a
//! `url(...)` pointing outward or a `<!ENTITY>` are all requests the renderer is willing to honour.
//! Nothing that can fetch, execute or expand may reach it, so what a picture is *allowed* to be is
//! decided here and nowhere else.
//!
//! **It is an allow-list scanner, not a search for forbidden strings.** A deny-list over raw text
//! loses to every trick the format has: a `<script>` hidden past a comment that the scanner read as
//! prose, an attribute quoted the other way, a name written in entities. So the markup is walked —
//! comments and CDATA skipped as opaque regions, every element name, attribute name and attribute
//! value read in turn — and anything not named on a list ends the parse. The lists are short on
//! purpose: shapes, paint and a local paint server, and no way to name anything outside the string.
//!
//! **Nothing here draws.** The result is bytes and a size; `crate::ui::a2ui` is what puts them on a
//! screen.

use std::collections::HashSet;

/// The largest markup this will look at, before it looks at it.
///
/// A cap on the input is the only bound that holds whatever the scan turns out to cost, which is
/// why it is checked before the first byte is read. An icon or a small illustration — which is what
/// this route is for — is orders of magnitude under it.
pub const MAX_BYTES: usize = 64 * 1024;

/// The most elements one picture may contain.
///
/// This is a cost cap on the scan itself rather than a judgement about the drawing: markup that is
/// small enough to pass [`MAX_BYTES`] can still be hundreds of thousands of empty tags.
pub const MAX_ELEMENTS: usize = 2000;

/// The elements a picture may be built from: shapes, grouping, and paint servers.
const ALLOWED_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "title",
    "desc",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "marker",
    "pattern",
];

/// The attributes a picture may carry: geometry, paint, type, and the ids that bind them.
///
/// `xmlns` is not here because it is allowed only on the root, and nowhere else has a use for it.
const ALLOWED_ATTRS: &[&str] = &[
    "d",
    "x",
    "y",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x1",
    "y1",
    "x2",
    "y2",
    "fx",
    "fy",
    "points",
    "width",
    "height",
    "viewBox",
    "transform",
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-opacity",
    "fill-opacity",
    "fill-rule",
    "opacity",
    "font-size",
    "font-family",
    "font-weight",
    "text-anchor",
    "dominant-baseline",
    "id",
    "class",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientUnits",
    "gradientTransform",
    "patternUnits",
    "spreadMethod",
];

/// The three prologue constructs that are refused wherever they appear.
///
/// A document type declaration is how an entity gets defined, an entity is how a fetch or a
/// billion-laughs expansion gets written, and a stylesheet instruction is how a fetch gets written
/// without an entity. None of them has a legitimate use in a picture an agent drew, and all three
/// are cheap to find before any structure is assumed.
const BANNED_PROLOGUE: &[&str] = &["<!doctype", "<!entity", "<?xml-stylesheet"];

/// Markup that passed, and the size its own `viewBox` declares.
///
/// The markup is kept private because it is only safe in the shape the scan left it in; [`bytes`]
/// is the one way out.
///
/// [`bytes`]: Sanitized::bytes
#[derive(Debug)]
pub struct Sanitized {
    markup: String,
    pub width: f32,
    pub height: f32,
}

impl Sanitized {
    /// The bytes to draw, with `currentColor` resolved to the colour the caller chose.
    ///
    /// The substitution happens here rather than at the call site so that `state/` never names a
    /// colour; the caller passes a hex string it got from a theme token.
    ///
    /// This makes the returned bytes palette-dependent, which is correct rather than unfortunate:
    /// `gpui::Image::from_bytes` identifies a picture by the hash of its bytes, so the window's
    /// image cache has to miss when the theme flips or the old colour stays on screen.
    pub fn bytes(self, current_color: &str) -> Vec<u8> {
        self.markup
            .replace("currentColor", current_color)
            .into_bytes()
    }
}

/// What the walk learned that only the whole document can answer.
struct Scan {
    /// Every `id` the markup defines, so a `url(#name)` can be checked against it.
    ids: HashSet<String>,
    /// Every local reference, with the attribute that made it, checked once the walk has finished
    /// because an id may be defined after the element that points at it.
    refs: Vec<(String, String)>,
    /// How many elements the walk saw at all.
    elements: usize,
}

/// Read agent-authored markup, or say in one sentence why it is refused.
///
/// The error is prose for whoever is looking at the payload, not a type to match on: every refusal
/// ends in the same place, a line saying the picture was not drawn and what in it stopped it.
pub fn sanitize(markup: &str) -> Result<Sanitized, String> {
    if markup.len() > MAX_BYTES {
        return Err(format!(
            "the markup is {} bytes, larger than the {MAX_BYTES} this reads",
            markup.len()
        ));
    }

    let lowered = markup.to_ascii_lowercase();
    for banned in BANNED_PROLOGUE {
        if lowered.contains(banned) {
            return Err(format!(
                "the markup contains `{banned}`, which can name something outside itself"
            ));
        }
    }
    if lowered.contains("data:") {
        return Err(
            "the markup contains `data:`, which carries a payload for a second decoder".to_string(),
        );
    }

    let scan = walk(markup)?;
    if scan.elements == 0 {
        return Err("the markup has no root `<svg>` element".to_string());
    }
    for (attr, id) in &scan.refs {
        if !scan.ids.contains(id) {
            return Err(format!(
                "`{attr}` refers to `url(#{id})`, but no element here carries `id=\"{id}\"`"
            ));
        }
    }

    // The helper lives in a Mermaid-named module because that is where the need for it first came
    // up; it reads a root `viewBox` and knows nothing about diagrams. A third caller is what would
    // earn moving it somewhere neutral.
    let (width, height) = crate::state::diagrams::view_box(markup)
        .ok_or_else(|| "the root `<svg>` declares no usable size in its `viewBox`".to_string())?;

    Ok(Sanitized {
        markup: markup.to_string(),
        width,
        height,
    })
}

/// Walk the markup once, refusing at the first thing that is not on a list.
///
/// The walk is deliberately dumber than an XML parser: it does not build a tree, does not resolve
/// namespaces and does not care whether tags nest. It only needs to see every name and every value
/// that the rasteriser will later see, and a parser that understood more would have more ways to
/// disagree with the one downstream.
fn walk(markup: &str) -> Result<Scan, String> {
    let bytes = markup.as_bytes();
    let mut scan = Scan {
        ids: HashSet::new(),
        refs: Vec::new(),
        elements: 0,
    };
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &markup[i..];

        // Opaque regions. Their contents are never elements, whatever they look like.
        if let Some(close) =
            opaque(rest, "<!--", "-->").or_else(|| opaque(rest, "<![CDATA[", "]]>"))
        {
            i += close?;
            continue;
        }
        // A closing tag, a declaration or a processing instruction: no names this cares about, and
        // the dangerous spellings were refused before the walk started.
        if rest.starts_with("</") || rest.starts_with("<!") || rest.starts_with("<?") {
            i += rest.find('>').ok_or_else(unclosed_tag)? + 1;
            continue;
        }

        let name_start = i + 1;
        let mut j = name_start;
        while j < bytes.len() && is_name_byte(bytes[j]) {
            j += 1;
        }
        if j == name_start {
            // A bare `<` in text. Nothing opens here.
            i += 1;
            continue;
        }
        let name = &markup[name_start..j];

        scan.elements += 1;
        if scan.elements > MAX_ELEMENTS {
            return Err(format!(
                "the markup has more than {MAX_ELEMENTS} elements, more than this will scan"
            ));
        }
        if scan.elements == 1 && name != "svg" {
            return Err(format!(
                "the markup opens with `<{name}>` rather than a root `<svg>`"
            ));
        }
        if !ALLOWED_ELEMENTS.contains(&name) {
            return Err(format!(
                "`<{name}>` is not an element a drawn picture may contain"
            ));
        }

        i = attributes(markup, bytes, j, name, &mut scan)?;
    }

    Ok(scan)
}

/// How far past `rest`'s start an opaque region ends, when `rest` opens one.
///
/// The outer `Option` says whether the region started at all; the inner `Result` is the refusal for
/// one that never ends, because markup that runs off the end of an unclosed comment is markup whose
/// remainder was never scanned.
fn opaque(rest: &str, open: &str, close: &str) -> Option<Result<usize, String>> {
    rest.starts_with(open).then(|| {
        rest.find(close)
            .map(|at| at + close.len())
            .ok_or_else(|| format!("the markup opens `{open}` and never closes it"))
    })
}

/// Read one element's attributes, returning where the tag ended.
fn attributes(
    markup: &str,
    bytes: &[u8],
    from: usize,
    element: &str,
    scan: &mut Scan,
) -> Result<usize, String> {
    let mut i = from;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(unclosed_tag());
        }
        if bytes[i] == b'>' {
            return Ok(i + 1);
        }
        if bytes[i] == b'/' || bytes[i] == b'=' {
            // A self-closing slash, or a stray `=` with no name in front of it.
            i += 1;
            continue;
        }

        let name_start = i;
        while i < bytes.len()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
            && !bytes[i].is_ascii_whitespace()
        {
            i += 1;
        }
        let attr = &markup[name_start..i];

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = "";
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(format!("`{attr}` opens a quoted value that never closes"));
                }
                value = &markup[value_start..i];
                i += 1;
            } else {
                let value_start = i;
                while i < bytes.len() && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = &markup[value_start..i];
            }
        }

        let permitted = ALLOWED_ATTRS.contains(&attr) || (attr == "xmlns" && element == "svg");
        if !permitted {
            return Err(format!(
                "`{attr}=` on `<{element}>` is not an attribute a drawn picture may carry"
            ));
        }
        if attr == "id" {
            scan.ids.insert(value.to_string());
        }
        references(attr, value, scan)?;
    }
}

/// Collect the local paint-server references in one value, refusing anything that points outward.
///
/// `url(#name)` is the legitimate case and the only one: a gradient, a clip path or a mask defined
/// in the same string. Every other `url(...)` is a fetch, whatever it is spelled as.
fn references(attr: &str, value: &str, scan: &mut Scan) -> Result<(), String> {
    let mut at = 0usize;
    while let Some(found) = value[at..].find("url(") {
        let open = at + found + "url(".len();
        let close = value[open..]
            .find(')')
            .ok_or_else(|| format!("`{attr}` has a `url(` that is never closed"))?;
        let inner = value[open..open + close]
            .trim()
            .trim_matches(['"', '\''])
            .trim();
        let id = inner.strip_prefix('#').ok_or_else(|| {
            format!("`{attr}` has `url({inner})`, which names something outside this markup")
        })?;
        scan.refs.push((attr.to_string(), id.to_string()));
        at = open + close + 1;
    }
    Ok(())
}

/// Bytes that may appear in an element or attribute name.
fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
}

/// The refusal for a tag that runs off the end of the markup.
fn unclosed_tag() -> String {
    "the markup opens a tag it never closes".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 16">
        <title>a shape</title>
        <path d="M2 2 L22 14" stroke="currentColor" stroke-width="2" fill="none"/>
        <circle cx="12" cy="8" r="3" fill="currentColor" fill-opacity="0.5"/>
    </svg>"#;

    /// One row per refusal the module states, each asserting the message names its own cause.
    #[test]
    fn refusals_name_what_they_refused() {
        let oversize = "a".repeat(MAX_BYTES + 1);
        let crowded = format!(
            r#"<svg viewBox="0 0 1 1">{}</svg>"#,
            "<g/>".repeat(MAX_ELEMENTS + 1)
        );
        let rows: &[(&str, &str, &str)] = &[
            ("oversize", &oversize, "larger than"),
            ("crowded", &crowded, "more than 2000 elements"),
            (
                "doctype",
                r#"<!DOCTYPE svg><svg viewBox="0 0 1 1"/>"#,
                "<!doctype",
            ),
            (
                "entity",
                r#"<svg viewBox="0 0 1 1"><!ENTITY x "y"/></svg>"#,
                "<!entity",
            ),
            (
                "stylesheet",
                r#"<?xml-stylesheet href="x.css"?><svg viewBox="0 0 1 1"/>"#,
                "<?xml-stylesheet",
            ),
            (
                "script",
                r#"<svg viewBox="0 0 1 1"><script>alert(1)</script></svg>"#,
                "<script>",
            ),
            (
                "foreign object",
                r#"<svg viewBox="0 0 1 1"><foreignObject/></svg>"#,
                "<foreignObject>",
            ),
            (
                "iframe",
                r#"<svg viewBox="0 0 1 1"><iframe/></svg>"#,
                "<iframe>",
            ),
            ("anchor", r#"<svg viewBox="0 0 1 1"><a/></svg>"#, "<a>"),
            (
                "style",
                r#"<svg viewBox="0 0 1 1"><style>*{}</style></svg>"#,
                "<style>",
            ),
            (
                "animate",
                r#"<svg viewBox="0 0 1 1"><animate/></svg>"#,
                "<animate>",
            ),
            ("use", r#"<svg viewBox="0 0 1 1"><use/></svg>"#, "<use>"),
            (
                "image",
                r#"<svg viewBox="0 0 1 1"><image/></svg>"#,
                "<image>",
            ),
            (
                "handler",
                r#"<svg viewBox="0 0 1 1"><circle onclick="x()"/></svg>"#,
                "onclick",
            ),
            (
                "href",
                r#"<svg viewBox="0 0 1 1"><circle href="http://x"/></svg>"#,
                "href",
            ),
            (
                "xlink href",
                r##"<svg viewBox="0 0 1 1"><circle xlink:href="#x"/></svg>"##,
                "xlink:href",
            ),
            (
                "xlink namespace",
                r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 1 1"/>"#,
                "xmlns:xlink",
            ),
            (
                "remote url",
                r#"<svg viewBox="0 0 1 1"><rect fill="url(http://x/#g)"/></svg>"#,
                "url(",
            ),
            (
                "data payload",
                r#"<svg viewBox="0 0 1 1"><rect fill="data:image/png;base64,AA"/></svg>"#,
                "data:",
            ),
            ("no root", "just some prose", "no root `<svg>`"),
            (
                "no size",
                r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#,
                "size",
            ),
        ];

        for (label, markup, needle) in rows {
            let error = sanitize(markup)
                .err()
                .unwrap_or_else(|| panic!("{label}: accepted markup that must be refused"));
            assert!(
                error.contains(needle),
                "{label}: `{error}` does not name `{needle}`"
            );
        }
    }

    #[test]
    fn a_clean_picture_passes_with_its_own_size() {
        let picture = sanitize(CLEAN).expect("clean markup");
        assert_eq!((picture.width, picture.height), (24.0, 16.0));
    }

    #[test]
    fn a_local_paint_server_is_the_one_url_allowed() {
        let with_id = r#"<svg viewBox="0 0 4 4"><defs><linearGradient id="grad" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="currentColor"/></linearGradient></defs><rect width="4" height="4" fill="url(#grad)"/></svg>"#;
        assert!(sanitize(with_id).is_ok());

        let without_id =
            r#"<svg viewBox="0 0 4 4"><rect width="4" height="4" fill="url(#grad)"/></svg>"#;
        let error = sanitize(without_id).expect_err("a dangling reference");
        assert!(error.contains("grad"), "{error}");
    }

    #[test]
    fn current_color_is_resolved_at_the_caller_s_colour() {
        let drawn = sanitize(CLEAN).expect("clean markup").bytes("#ff8800");
        let drawn = String::from_utf8(drawn).expect("utf-8");
        assert!(!drawn.contains("currentColor"));
        assert_eq!(drawn.matches("#ff8800").count(), 2);
    }

    /// The pair that proves the scan is a walk rather than a search for a string: the same six
    /// characters pass inside an opaque region and are refused outside one.
    #[test]
    fn a_script_hidden_in_a_comment_is_prose_and_one_outside_is_not() {
        let commented = r#"<svg viewBox="0 0 4 4"><!-- <script>alert(1)</script> --><rect width="4" height="4"/></svg>"#;
        assert!(sanitize(commented).is_ok(), "a comment is not markup");

        let cdata =
            r#"<svg viewBox="0 0 4 4"><desc><![CDATA[<script>alert(1)</script>]]></desc></svg>"#;
        assert!(sanitize(cdata).is_ok(), "CDATA is not markup");

        let real = r#"<svg viewBox="0 0 4 4"><!-- harmless --><script>alert(1)</script></svg>"#;
        let error = sanitize(real).expect_err("a real script element");
        assert!(error.contains("<script>"), "{error}");
    }

    #[test]
    fn an_unclosed_comment_swallows_the_rest_and_is_refused() {
        let error = sanitize(r#"<svg viewBox="0 0 4 4"><!-- never ends"#).expect_err("unclosed");
        assert!(error.contains("<!--"), "{error}");
    }
}
