//! The portable output path contract, syntax only (D8): `renderers/shared/portable-path.mjs`
//! `validatePortablePath` with the `output` profile, and the two things built on it.
//!
//! - the schema-stage wrapper: after the schema passes, `meta.output` must satisfy the contract or
//!   `schema/portablePath` is reported (see `schema::validate`);
//! - the V2 stage: [`authored_output_diagnostic`] is `validateAuthoredOutputPath` without the
//!   resolution half (symlinks, inside-cwd, `output/meta-*-resolved`), which needs a filesystem and
//!   is dropped because nothing here writes HTML.
//!
//! Lone surrogates (`unpaired-surrogate`) cannot occur in a Rust `str`; a document that carries one
//! never gets past `serde_json`, which reports `input/json-parse`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::diag::{Diagnostic, Subject};

/// Why a path was refused. `code()` is `portable-path/<reason>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortablePathError {
    pub reason: &'static str,
    pub message: &'static str,
    pub segment: Option<String>,
    pub segment_index: Option<usize>,
    pub character: Option<char>,
    pub utf8_bytes: Option<usize>,
    pub utf16_code_units: Option<usize>,
    pub limit: Option<usize>,
}

impl PortablePathError {
    fn new(reason: &'static str, message: &'static str) -> Self {
        PortablePathError {
            reason,
            message,
            segment: None,
            segment_index: None,
            character: None,
            utf8_bytes: None,
            utf16_code_units: None,
            limit: None,
        }
    }

    fn at(mut self, segment: &str, index: usize) -> Self {
        self.segment = Some(segment.to_owned());
        self.segment_index = Some(index);
        self
    }

    pub fn code(&self) -> String {
        format!("portable-path/{}", self.reason)
    }

    /// The evidence `authoredOutputDiagnostic` records.
    pub fn evidence(&self) -> Map<String, Value> {
        let mut evidence = Map::new();
        evidence.insert("reason".into(), self.reason.into());
        if let Some(index) = self.segment_index {
            evidence.insert("segmentIndex".into(), index.into());
        }
        if let Some(segment) = &self.segment {
            evidence.insert("segment".into(), segment.clone().into());
        }
        if let Some(bytes) = self.utf8_bytes {
            evidence.insert("utf8Bytes".into(), bytes.into());
        }
        if let Some(units) = self.utf16_code_units {
            evidence.insert("utf16CodeUnits".into(), units.into());
        }
        if let Some(limit) = self.limit {
            evidence.insert("limit".into(), limit.into());
        }
        evidence
    }
}

struct Patterns {
    drive_absolute: Regex,
    drive_relative: Regex,
    uri: Regex,
    reserved: Regex,
    short_name: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        drive_absolute: Regex::new(r"^[A-Za-z]:[\\/]").expect("regex"),
        drive_relative: Regex::new(r"^[A-Za-z]:").expect("regex"),
        uri: Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").expect("regex"),
        reserved: Regex::new(
            r"(?i)^(?:con|prn|aux|nul|conin\$|conout\$|com[1-9¹²³]|lpt[1-9¹²³])(?:\.|$)",
        )
        .expect("regex"),
        short_name: Regex::new(r"~[1-9][0-9]*(?:\.|$)").expect("regex"),
    })
}

/// `validatePortablePath(value, { profile: 'output' })`: the first violation, in Archify's order.
pub fn validate(value: &str) -> Result<(), Box<PortablePathError>> {
    let p = patterns();
    let fail = |reason, message| Err(Box::new(PortablePathError::new(reason, message)));
    if value.is_empty() {
        return fail("empty", "Portable path must not be empty.");
    }
    if p.drive_absolute.is_match(value) || value.starts_with('/') || value.starts_with('\\') {
        return fail("absolute", "Portable path must be relative.");
    }
    if p.drive_relative.is_match(value) {
        return fail(
            "drive-relative",
            "Portable path must not use a drive-relative Windows path.",
        );
    }
    if p.uri.is_match(value) {
        return fail("uri", "Portable path must not be a URI.");
    }
    if value.contains('\\') {
        return fail(
            "backslash",
            "Portable path must use forward slashes as separators.",
        );
    }
    for (index, segment) in value.split('/').enumerate() {
        let err = |reason, message| {
            Err(Box::new(
                PortablePathError::new(reason, message).at(segment, index),
            ))
        };
        if segment.is_empty() {
            return err(
                "empty-segment",
                "Portable path must not contain empty segments.",
            );
        }
        if segment == "." || segment == ".." {
            return err(
                "dot-segment",
                "Portable path must not contain dot segments.",
            );
        }
        if let Some(c) = segment.chars().find(|c| c.is_control()) {
            let mut e = PortablePathError::new(
                "control",
                "Portable path must not contain control characters.",
            )
            .at(segment, index);
            e.character = Some(c);
            return Err(Box::new(e));
        }
        if segment.contains(':') {
            let mut e = PortablePathError::new(
                "windows-ads",
                "Portable path must not select a Windows alternate data stream.",
            )
            .at(segment, index);
            e.character = Some(':');
            return Err(Box::new(e));
        }
        if let Some(c) = segment.chars().find(|c| "<>\"|?*".contains(*c)) {
            let mut e = PortablePathError::new(
                "windows-invalid-character",
                "Portable path contains a character that is invalid in Windows file names.",
            )
            .at(segment, index);
            e.character = Some(c);
            return Err(Box::new(e));
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return err(
                "windows-trailing-dot-space",
                "Portable path segments must not end with a dot or space.",
            );
        }
        if p.reserved.is_match(segment) {
            return err(
                "windows-reserved-name",
                "Portable path must not use a reserved Windows device name.",
            );
        }
        if p.short_name.is_match(segment) {
            return err(
                "windows-short-name",
                "Portable path must not use a Windows 8.3 short-name shape.",
            );
        }
        let utf8 = segment.len();
        let utf16 = segment.encode_utf16().count();
        if utf8 > 255 || utf16 > 255 {
            let mut e = PortablePathError::new(
                "component-too-long",
                "Portable path segments must fit both UTF-8 and UTF-16 filesystem component limits.",
            )
            .at(segment, index);
            e.utf8_bytes = Some(utf8);
            e.utf16_code_units = Some(utf16);
            e.limit = Some(255);
            return Err(Box::new(e));
        }
    }
    Ok(())
}

/// `authoredOutputDiagnostic`: `output/meta-absolute` or `output/meta-path-syntax`.
pub fn authored_output_diagnostic(error: &PortablePathError, raw: &str) -> Diagnostic {
    let (code, message) = if error.reason == "absolute" {
        (
            "output/meta-absolute",
            "meta.output must be a relative path resolved from the current working directory.",
        )
    } else {
        (
            "output/meta-path-syntax",
            "meta.output must be a portable POSIX-relative path.",
        )
    };
    Diagnostic::error(code, message)
        .with_subject(output_subject(raw))
        .with_evidence(error.evidence())
        .with_fixes([
            "set meta.output to a portable relative .html path such as reports/diagram.html",
        ])
}

fn output_subject(raw: &str) -> Subject {
    Subject::default()
        .with_extra("output", raw.into())
        .with_path("/meta/output")
}

/// `path.posix.extname(raw).toLowerCase()`.
fn extension(raw: &str) -> String {
    let base = raw.rsplit('/').next().unwrap_or(raw);
    match base.rfind('.') {
        Some(0) | None => String::new(),
        Some(at) => base[at..].to_lowercase(),
    }
}

/// Stage V2, syntax half (D8): the portable-path contract, then the `.html` extension. `None`
/// when `meta.output` is acceptable.
pub fn validate_authored_output(raw: &str) -> Option<Diagnostic> {
    if let Err(error) = validate(raw) {
        return Some(authored_output_diagnostic(&error, raw));
    }
    (extension(raw) != ".html").then(|| {
        Diagnostic::error(
            "output/meta-extension",
            "meta.output must target an .html file.",
        )
        .with_subject(output_subject(raw))
        .with_fixes(["change meta.output to a portable path ending in .html"])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(value: &str) -> &'static str {
        validate(value).expect_err(value).reason
    }

    #[test]
    fn accepts_ordinary_relative_paths() {
        for ok in [
            "a.html",
            "out/web/app.html",
            "a b/c.html",
            "caf\u{e9}/x.html",
            "con-sole/x.html",
        ] {
            assert_eq!(validate(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn rejects_in_archifys_order() {
        for (value, want) in [
            ("", "empty"),
            ("/abs.html", "absolute"),
            ("\\abs.html", "absolute"),
            ("C:\\x.html", "absolute"),
            ("C:/x.html", "absolute"),
            ("C:x.html", "drive-relative"),
            ("https://x.html", "uri"),
            ("a\\b.html", "backslash"),
            ("a//b.html", "empty-segment"),
            ("a/.html/", "empty-segment"),
            ("./a.html", "dot-segment"),
            ("a/../b.html", "dot-segment"),
            ("a\u{7}.html", "control"),
            ("a\u{85}.html", "control"),
            ("a/b:c.html", "windows-ads"),
            ("a<b.html", "windows-invalid-character"),
            ("a?.html", "windows-invalid-character"),
            ("a/b .html/c ", "windows-trailing-dot-space"),
            ("a.html.", "windows-trailing-dot-space"),
            ("CON.html", "windows-reserved-name"),
            ("x/nul", "windows-reserved-name"),
            ("x/Lpt\u{b9}.html", "windows-reserved-name"),
            ("x/conin$", "windows-reserved-name"),
            ("PROGRA~1.html", "windows-short-name"),
            ("a/b~12", "windows-short-name"),
        ] {
            assert_eq!(reason(value), want, "{value:?}");
        }
    }

    #[test]
    fn component_limits_count_bytes_and_utf16_units() {
        assert!(validate(&format!("{}.html", "a".repeat(250))).is_ok());
        let long = format!("{}.html", "a".repeat(251));
        let error = validate(&long).unwrap_err();
        assert_eq!(error.reason, "component-too-long");
        assert_eq!(
            (error.utf8_bytes, error.utf16_code_units, error.limit),
            (Some(256), Some(256), Some(255))
        );
        // 100 x 3-byte characters: 300 UTF-8 bytes, 100 UTF-16 units.
        assert_eq!(
            reason(&format!("{}.html", "\u{20ac}".repeat(100))),
            "component-too-long"
        );
        // Astral: 2 UTF-16 units, 4 bytes each; 128 of them is 256 units.
        assert_eq!(
            reason(&format!("{}.html", "\u{1F600}".repeat(64))),
            "component-too-long"
        );
    }

    #[test]
    fn code_and_evidence() {
        let error = validate("a/b:c.html").unwrap_err();
        assert_eq!(error.code(), "portable-path/windows-ads");
        let evidence = error.evidence();
        assert_eq!(evidence["reason"], "windows-ads");
        assert_eq!(evidence["segmentIndex"], 1);
        assert_eq!(evidence["segment"], "b:c.html");
    }

    #[test]
    fn v2_diagnostics_match_archify() {
        assert!(validate_authored_output("reports/x.html").is_none());
        assert!(validate_authored_output("reports/x.HTML").is_none());

        let ext = validate_authored_output("a.htm").unwrap();
        assert_eq!(ext.code, "output/meta-extension");
        assert_eq!(ext.message, "meta.output must target an .html file.");
        assert_eq!(ext.subject.path.as_deref(), Some("/meta/output"));
        assert_eq!(ext.subject.extra["output"], "a.htm");
        assert_eq!(
            ext.supported_fixes,
            ["change meta.output to a portable path ending in .html"]
        );
        assert_eq!(
            validate_authored_output(".html").unwrap().code,
            "output/meta-extension"
        );
        assert_eq!(
            validate_authored_output("x/y").unwrap().code,
            "output/meta-extension"
        );

        let abs = validate_authored_output("/tmp/x.html").unwrap();
        assert_eq!(abs.code, "output/meta-absolute");
        assert_eq!(abs.evidence["reason"], "absolute");
        let syntax = validate_authored_output("a/../x.html").unwrap();
        assert_eq!(syntax.code, "output/meta-path-syntax");
        assert_eq!(
            syntax.message,
            "meta.output must be a portable POSIX-relative path."
        );
        assert_eq!(syntax.evidence["segment"], "..");
    }
}
