//! Diagnostics: `Diagnostic`, `Severity`, `Subject`, `Receipt`, text format (P1.1).
//!
//! The wire shape is Archify's `normalizedDiagnostic` (00 §3.2, 01 §3.3): `code` is exactly what
//! Archify emits (D7), and `subject.rule` carries the 00 rule id alongside. Staging, quality
//! severities and the rules themselves live elsewhere; this module is the shape, the text format and
//! the two post-filters (suppress, dedup by message).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The code families (00 §3.2). A code is `<family>/<kebab-name>`.
pub const FAMILIES: &[&str] = &[
    "schema",
    "input",
    "output",
    "portable-path",
    "relationship",
    "i18n",
    "engineering",
    "repository-evidence",
    "layout",
    "clean-flow",
    "composition",
    "legend",
    "workflow",
    "migration",
    "delta",
    "artifact",
    "viewer",
    "delivery",
    "finalize",
    "cli",
    "opener",
    "internal",
];

/// The code a diagnostic without one is given (never a stack trace).
pub const UNCLASSIFIED: &str = "internal/unclassified";

/// CLI exit codes (00 §3.6).
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

/// Anything that is not `warning` is an error, on the wire and when reading one back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    #[serde(other)]
    Error,
}

/// What a diagnostic is about. Every field is optional on the wire; `extra` holds the
/// rule-specific ones (`edge`, `from`, `to`, `output`, ...), flattened.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    #[serde(
        rename = "diagramType",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub diagram_type: Option<String>,
    /// A JSON Pointer into the document (`/` is the root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The nearest enclosing `id`, else `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Signed: `engineering/deployment-boundary-kind` reports `-1` (no boundary exists to point at).
    pub index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The 00 rule id (`graph/duplicate-node-id`, `geom/node-overlap`, ...), D7. Not an Archify
    /// field: it lets a caller tell † rules apart that share a wire code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Subject {
    pub fn of(diagram_type: &str) -> Self {
        Subject {
            diagram_type: Some(diagram_type.to_owned()),
            ..Subject::default()
        }
    }
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }
    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }
    pub fn with_extra(mut self, key: &str, value: Value) -> Self {
        self.extra.insert(key.to_owned(), value);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(default)]
    pub subject: Subject,
    #[serde(default)]
    pub evidence: Map<String, Value>,
    #[serde(rename = "supportedFixes", default)]
    pub supported_fixes: Vec<String>,
    /// A causal diagnostic hides the listed derivative codes (see [`apply_suppresses`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppresses: Option<Vec<String>>,
}

impl Diagnostic {
    /// Normalised like `normalizedDiagnostic`: an empty code is `internal/unclassified`, the message
    /// is trimmed (and never empty).
    pub fn new(code: &str, severity: Severity, message: &str) -> Self {
        let message = message.trim();
        Diagnostic {
            code: if code.trim().is_empty() {
                UNCLASSIFIED.to_owned()
            } else {
                code.to_owned()
            },
            severity,
            message: if message.is_empty() {
                "Archify could not classify this failure.".to_owned()
            } else {
                message.to_owned()
            },
            subject: Subject::default(),
            evidence: Map::new(),
            supported_fixes: Vec::new(),
            suppresses: None,
        }
    }

    pub fn error(code: &str, message: &str) -> Self {
        Self::new(code, Severity::Error, message)
    }

    pub fn warning(code: &str, message: &str) -> Self {
        Self::new(code, Severity::Warning, message)
    }

    pub fn with_subject(mut self, subject: Subject) -> Self {
        self.subject = subject;
        self
    }

    pub fn with_evidence(mut self, evidence: Map<String, Value>) -> Self {
        self.evidence = evidence;
        self
    }

    /// Fixes are trimmed, emptied ones dropped, duplicates removed (first wins).
    pub fn with_fixes<I, S>(mut self, fixes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut seen = HashSet::new();
        self.supported_fixes = fixes
            .into_iter()
            .map(|fix| fix.as_ref().trim().to_owned())
            .filter(|fix| !fix.is_empty() && seen.insert(fix.clone()))
            .collect();
        self
    }

    pub fn with_suppresses<I, S>(mut self, codes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut seen = HashSet::new();
        self.suppresses = Some(
            codes
                .into_iter()
                .map(|code| code.as_ref().trim().to_owned())
                .filter(|code| !code.is_empty() && seen.insert(code.clone()))
                .collect(),
        );
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity != Severity::Warning
    }

    /// The part of the code before the first `/`.
    pub fn family(&self) -> &str {
        family_of(&self.code)
    }

    /// Whether the code's family is one of [`FAMILIES`].
    pub fn has_known_family(&self) -> bool {
        FAMILIES.contains(&self.family())
    }

    /// What goldens compare (`code`, `subject.path`, `severity`), 01 §3.3: "never by count".
    pub fn key(&self) -> (&str, Option<&str>, Severity) {
        (&self.code, self.subject.path.as_deref(), self.severity)
    }

    /// `[code] message Fix: a; b.` (`formatDiagnostics`).
    pub fn line(&self) -> String {
        let fix = if self.supported_fixes.is_empty() {
            String::new()
        } else {
            format!(" Fix: {}.", self.supported_fixes.join("; "))
        };
        format!("[{}] {}{}", self.code, self.message, fix)
    }
}

pub fn family_of(code: &str) -> &str {
    code.split_once('/').map_or(code, |(family, _)| family)
}

/// The CLI's text format (stderr, exit 1): the error, then one line per diagnostic.
pub fn format_text(error: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = vec![error.to_owned()];
    out.extend(diagnostics.iter().map(Diagnostic::line));
    out.join("\n")
}

/// JSON mode deduplicates by message: only the first occurrence is kept.
pub fn dedup_by_message(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut seen = HashSet::new();
    diagnostics
        .into_iter()
        .filter(|d| seen.insert(d.message.clone()))
        .collect()
}

/// The suppress post-filter: a diagnostic whose code is listed in another's `suppresses` is
/// dropped. The carrier stays, including its `suppresses` field.
pub fn apply_suppresses(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let hidden: HashSet<String> = diagnostics
        .iter()
        .flat_map(|d| d.suppresses.iter().flatten().cloned())
        .collect();
    if hidden.is_empty() {
        return diagnostics;
    }
    diagnostics
        .into_iter()
        .filter(|d| !hidden.contains(&d.code))
        .collect()
}

/// Both post-filters, in Archify's order: causal suppression, then dedup by message.
pub fn post_filter(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    dedup_by_message(apply_suppresses(diagnostics))
}

/// JavaScript `Math.round`: halves round toward +infinity (`round(-2.5) == -2`), unlike
/// `f64::round`. `-0.0` is kept for inputs in `[-0.5, 0)`, as in JS.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    if rounded == 0.0 && x < 0.0 {
        -0.0
    } else {
        rounded
    }
}

/// A JSON number the way JS prints it: whole values carry no `.0`.
pub fn json_num(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15 {
        Value::from(x as i64)
    } else {
        serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
    }
}

/// Where a failing `validate` stopped (the receipt's `stage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Input,
    Render,
    Check,
    Arguments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextAction {
    pub command: String,
    pub arguments: Vec<String>,
}

/// The `validate --json` receipt (00 §3.6). Success carries `candidate .. composition`; failure
/// carries `stage`, `error` and `diagnostics`. The gate blocks (`checks`, `composition`) stay
/// untyped here: later phases fill them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub ok: bool,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    #[serde(rename = "type")]
    pub doc_type: String,
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<Candidate>,
    #[serde(
        rename = "candidateFrozen",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub candidate_frozen: Option<bool>,
    #[serde(
        rename = "nextAction",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub next_action: Option<NextAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Value>,
    #[serde(
        rename = "engineeringProfile",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub engineering_profile: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checker: Option<Value>,
}

impl Receipt {
    fn base(command: &str, doc_type: &str, input: &str, ok: bool) -> Self {
        Receipt {
            schema_version: 1,
            ok,
            command: command.to_owned(),
            stage: None,
            doc_type: doc_type.to_owned(),
            input: input.to_owned(),
            error: None,
            diagnostics: Vec::new(),
            candidate: None,
            candidate_frozen: None,
            next_action: None,
            checks: Vec::new(),
            composition: None,
            engineering_profile: None,
            checker: None,
        }
    }

    /// A failing receipt. `diagnostics` go through [`post_filter`].
    pub fn failure(
        command: &str,
        stage: Stage,
        doc_type: &str,
        input: &str,
        error: &str,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        let mut receipt = Self::base(command, doc_type, input, false);
        receipt.stage = Some(stage);
        receipt.error = Some(error.to_owned());
        receipt.diagnostics = post_filter(diagnostics);
        receipt
    }

    /// A passing receipt with the frozen candidate; the caller fills `checks` and `composition`.
    pub fn success(command: &str, doc_type: &str, input: &str, candidate: Candidate) -> Self {
        let mut receipt = Self::base(command, doc_type, input, true);
        receipt.candidate = Some(candidate);
        receipt.candidate_frozen = Some(true);
        receipt
    }

    /// 0 success, 1 the requested thing failed, 2 usage (`stage: arguments`), 00 §3.6.
    pub fn exit_code(&self) -> i32 {
        match (self.ok, self.stage) {
            (true, _) => EXIT_OK,
            (false, Some(Stage::Arguments)) => EXIT_USAGE,
            (false, _) => EXIT_FAILED,
        }
    }

    /// Text mode. Failure: the error and one `[code] message Fix: ...` line per diagnostic.
    /// Success: `ok <type> <input> (N artifact checks; composition <profile>: E errors, W warnings)`.
    pub fn text(&self) -> String {
        if !self.ok {
            return format_text(self.error.as_deref().unwrap_or_default(), &self.diagnostics);
        }
        let mut line = format!("ok {} {}", self.doc_type, self.input);
        if let Some(composition) = &self.composition {
            let count = |key: &str| {
                composition
                    .pointer(&format!("/summary/{key}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            let profile = composition
                .get("profile")
                .and_then(Value::as_str)
                .unwrap_or("advisory");
            line.push_str(&format!(
                " ({} artifact checks; composition {profile}: {} errors, {} warnings)",
                self.checks.len(),
                count("errors"),
                count("warnings"),
            ));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn diag(code: &str, message: &str) -> Diagnostic {
        Diagnostic::error(code, message)
    }

    #[test]
    fn families_cover_every_code_family_in_the_spec() {
        assert_eq!(FAMILIES.len(), 22);
        assert!(FAMILIES.iter().all(|f| !f.contains('/')));
        assert!(diag("portable-path/empty", "x").has_known_family());
        assert!(!diag("bogus/thing", "x").has_known_family());
        assert_eq!(family_of("schema/required"), "schema");
        assert_eq!(family_of("nofamily"), "nofamily");
    }

    #[test]
    fn empty_code_and_message_are_normalised() {
        let d = Diagnostic::new("", Severity::Error, "  \n");
        assert_eq!(d.code, UNCLASSIFIED);
        assert_eq!(d.message, "Archify could not classify this failure.");
        assert_eq!(diag("a/b", "  hi  ").message, "hi");
    }

    #[test]
    fn severity_defaults_to_error_on_the_wire() {
        let sev = |s: &str| serde_json::from_value::<Severity>(json!(s)).unwrap();
        assert_eq!(sev("warning"), Severity::Warning);
        assert_eq!(sev("error"), Severity::Error);
        assert_eq!(sev("fatal"), Severity::Error);
        assert_eq!(
            serde_json::to_value(Severity::Warning).unwrap(),
            json!("warning")
        );
    }

    #[test]
    fn wire_shape_matches_archify() {
        let d = diag("schema/additionalProperties", "/nodes/3 bad")
            .with_subject(
                Subject::of("workflow")
                    .with_path("/nodes/3")
                    .with_identity("router")
                    .with_rule("schema/additionalProperties")
                    .with_extra("edge", json!("e1")),
            )
            .with_fixes(["remove unsupported property \"colour\""])
            .with_suppresses(["workflow/short-edge"]);
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(
            v,
            json!({
                "code": "schema/additionalProperties",
                "severity": "error",
                "message": "/nodes/3 bad",
                "subject": {
                    "diagramType": "workflow", "path": "/nodes/3", "identity": "router",
                    "rule": "schema/additionalProperties", "edge": "e1"
                },
                "evidence": {},
                "supportedFixes": ["remove unsupported property \"colour\""],
                "suppresses": ["workflow/short-edge"],
            })
        );
        let back: Diagnostic = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn absent_optionals_are_omitted() {
        let v = serde_json::to_value(diag("cli/usage", "m")).unwrap();
        assert_eq!(
            v,
            json!({"code": "cli/usage", "severity": "error", "message": "m",
                   "subject": {}, "evidence": {}, "supportedFixes": []})
        );
    }

    #[test]
    fn fixes_are_trimmed_and_deduplicated() {
        let d = diag("a/b", "m").with_fixes([" one ", "", "one", "two"]);
        assert_eq!(d.supported_fixes, ["one", "two"]);
    }

    #[test]
    fn text_format_is_code_message_fix() {
        let a = diag("schema/required", "/ missing").with_fixes(["add a", "add b"]);
        let b = diag("layout/constraint", "overlap");
        assert_eq!(a.line(), "[schema/required] / missing Fix: add a; add b.");
        assert_eq!(
            format_text("dataflow schema validation failed", &[a, b]),
            "dataflow schema validation failed\n[schema/required] / missing Fix: add a; add b.\n[layout/constraint] overlap"
        );
        assert_eq!(format_text("only", &[]), "only");
    }

    #[test]
    fn dedup_keeps_the_first_message() {
        let out = dedup_by_message(vec![
            diag("a/x", "same"),
            diag("a/y", "same"),
            diag("a/z", "other"),
        ]);
        let codes: Vec<_> = out.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["a/x", "a/z"]);
    }

    #[test]
    fn suppress_hides_the_listed_codes_only() {
        let cause = diag("workflow/column-capacity", "cause")
            .with_suppresses(["workflow/short-edge", "clean-flow/endpoint-side-direction"]);
        let out = apply_suppresses(vec![
            diag("workflow/short-edge", "a"),
            cause.clone(),
            diag("clean-flow/endpoint-side-direction", "b"),
            diag("layout/constraint", "c"),
        ]);
        let codes: Vec<_> = out.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["workflow/column-capacity", "layout/constraint"]);
        assert_eq!(out[0], cause);
        // Nothing suppressing: untouched.
        assert_eq!(apply_suppresses(vec![diag("a/b", "x")]).len(), 1);
    }

    #[test]
    fn post_filter_suppresses_then_dedups() {
        let out = post_filter(vec![
            diag("a/x", "dup"),
            diag("a/y", "dup"),
            diag("b/c", "cause").with_suppresses(["a/x"]),
        ]);
        let codes: Vec<_> = out.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["a/y", "b/c"]);
    }

    #[test]
    fn js_round_follows_math_round() {
        for (x, want) in [
            (0.5, 1.0),
            (1.5, 2.0),
            (2.5, 3.0),
            (-0.5, 0.0),
            (-1.5, -1.0),
            (-2.5, -2.0),
            (2.4, 2.0),
            (-2.6, -3.0),
            (0.49999999999999994, 0.0),
            (1e15 + 0.5, 1e15 + 1.0),
        ] {
            assert_eq!(js_round(x), want, "round({x})");
        }
        assert!(js_round(-0.4).is_sign_negative());
        assert!(js_round(f64::NAN).is_nan());
        assert_eq!(js_round(f64::INFINITY), f64::INFINITY);
        // The thing the map warns about: Rust rounds half away from zero.
        assert_ne!((-2.5f64).round(), js_round(-2.5));
    }

    #[test]
    fn json_num_drops_the_trailing_zero() {
        assert_eq!(json_num(360.0).to_string(), "360");
        assert_eq!(json_num(0.5).to_string(), "0.5");
        assert_eq!(json_num(f64::NAN), Value::Null);
    }

    #[test]
    fn receipts_have_archify_shape_and_exit_codes() {
        let failure = Receipt::failure(
            "validate",
            Stage::Render,
            "dataflow",
            "/tmp/a.json",
            "meta.output must target an .html file.",
            vec![
                diag(
                    "output/meta-extension",
                    "meta.output must target an .html file.",
                )
                .with_fixes(["change meta.output to a portable path ending in .html"]),
            ],
        );
        assert_eq!(failure.exit_code(), 1);
        let v = serde_json::to_value(&failure).unwrap();
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        for k in [
            "schemaVersion",
            "ok",
            "command",
            "stage",
            "type",
            "input",
            "error",
            "diagnostics",
        ] {
            assert!(keys.contains(&k.to_owned()), "{k} in {keys:?}");
        }
        assert!(v.get("candidate").is_none());
        assert_eq!(v["stage"], json!("render"));
        assert_eq!(
            failure.text(),
            "meta.output must target an .html file.\n[output/meta-extension] meta.output must target an .html file. Fix: change meta.output to a portable path ending in .html."
        );

        let mut usage = failure.clone();
        usage.stage = Some(Stage::Arguments);
        assert_eq!(usage.exit_code(), 2);

        let mut ok = Receipt::success(
            "validate",
            "dataflow",
            "/tmp/a.json",
            Candidate {
                path: "/tmp/a.json".into(),
                sha256: "ab".into(),
                bytes: 5,
            },
        );
        assert_eq!(ok.exit_code(), 0);
        ok.checks = vec![json!({"name": "single_svg", "ok": true}); 9];
        ok.composition =
            Some(json!({"profile": "showcase", "summary": {"errors": 0, "warnings": 0}}));
        assert_eq!(
            ok.text(),
            "ok dataflow /tmp/a.json (9 artifact checks; composition showcase: 0 errors, 0 warnings)"
        );
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["candidateFrozen"], json!(true));
        assert!(
            v.get("stage").is_none() && v.get("error").is_none() && v.get("diagnostics").is_none()
        );
        let back: Receipt = serde_json::from_value(v).unwrap();
        assert_eq!(back, ok);
    }

    #[test]
    fn failure_receipt_runs_the_post_filters() {
        let r = Receipt::failure(
            "validate",
            Stage::Render,
            "workflow",
            "x",
            "e",
            vec![
                diag("workflow/short-edge", "derived"),
                diag("workflow/column-capacity", "cause").with_suppresses(["workflow/short-edge"]),
                diag("workflow/column-capacity", "cause"),
            ],
        );
        assert_eq!(r.diagnostics.len(), 1);
    }

    #[test]
    fn keys_compare_code_path_severity() {
        let d =
            diag("schema/type", "m").with_subject(Subject::of("sequence").with_path("/messages/0"));
        assert_eq!(
            d.key(),
            ("schema/type", Some("/messages/0"), Severity::Error)
        );
    }
}
