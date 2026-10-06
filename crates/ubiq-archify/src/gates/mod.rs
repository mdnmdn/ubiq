//! Quality gates (P2.5): `clean-flow/*`, `composition/*` and the per-type geometry rules, ported from
//! `renderers/shared/geometry.mjs` and each renderer's `validate*`.
//!
//! # Shape
//!
//! A gate reads *routed relations* ([`Rel`]) and *label rects* ([`LabelRect`]), never a document, so
//! the architecture, sequence, lifecycle and workflow layouts reuse them (P3+). Each gate writes into
//! a [`Sink`], which mirrors what Archify does with a problem: the `clean*` gates both *record* a rich
//! diagnostic (`recordDiagnostic`) and return its message as a *problem*; the plain sentences of
//! `validate*` are only problems. In JSON mode Archify reports the recorded diagnostics first (in the
//! order recorded), then the remaining problems as `layout/constraint`, deduplicated by message.
//! [`Sink::finish`] does the same, and keeps the problem order for the receipt's `error` text.
//!
//! # Severity by profile (00 section 3.3, `geometry.mjs`)
//!
//! [`Gate`] is the profile the renderer sees: `--quality` (the CLI sets `ARCHIFY_QUALITY_PROFILE`),
//! else the authored `meta.quality_profile`, else none. It is *not* the reported profile, which
//! defaults to `standard`. Per rule (dataflow opts into none of the workflow switches):
//!
//! | rule | none | advisory/standard | showcase |
//! |---|---|---|---|
//! | `clean-flow/*` | error | error | error |
//! | `composition/container-border-run` | silent | error | error |
//! | `proper-crossing`, `ambiguous-corridor`, `arrowhead-collision` | silent | silent (warning in workflow v2) | error |
//! | `micro-segment`, `short-interior-segment`, `label-route-clearance`, `label-canvas-containment` | silent | silent | error |
//!
//! `advisory` as a *value* is a profile like any other (`!profile` is the only test `border-run`
//! makes), so `--quality advisory` enables border runs; the document schema only allows the other two.

use serde_json::{Map, Value};

use crate::diag::{Diagnostic, Subject, dedup_by_message, js_round};
use crate::geom::{Pt, Rect, Side};

pub mod architecture;
pub mod artifact;
pub mod clean_flow;
pub mod composition;
pub mod dataflow;
pub mod lifecycle;
pub mod sequence;
pub mod suggest;
pub mod workflow_v1;
pub mod workflow_v2;

/// The profile a gate sees (see the module doc). `None` is "no profile at all".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate(pub Option<&'static str>);

impl Gate {
    pub fn showcase(self) -> bool {
        self.0 == Some("showcase")
    }

    /// Any profile at all (`if (!profile) return []`).
    pub fn declared(self) -> bool {
        self.0.is_some()
    }

    /// `showcase` or `standard` as the composition wording prints it.
    pub fn word(self) -> &'static str {
        if self.showcase() { "showcase" } else { "standard" }
    }
}

/// The authored routing controls a relation can carry (`AUTHORED_PLAN_KEYS`), in Archify's order.
pub const PLAN_KEYS: [&str; 11] = [
    "route", "via", "channelX", "channelY", "fromSide", "toSide", "bias", "labelAt", "labelDx", "labelDy",
    "labelSegment",
];

/// A relationship whose route is known: a flow, connection, message or transition.
#[derive(Clone, Debug, PartialEq)]
pub struct Rel {
    /// The index in the collection (`flows[i]`).
    pub index: usize,
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    pub label: String,
    /// The route as `pathFor` returns it (not normalised).
    pub points: Vec<Pt>,
    /// Which of [`PLAN_KEYS`] are authored on it.
    pub controls: Vec<&'static str>,
    /// Arrow stroke width: `relation.width || (variant === 'emphasis' ? 1.8 : 1.5)`.
    pub arrow_width: f64,
    /// The side the first segment must honour, and whether it was authored or inferred. `None`
    /// where the route is authored geometry without a side (nothing to check).
    pub from_side: CheckedSide,
    /// The same for the last segment.
    pub to_side: CheckedSide,
}

/// A side to check and where it came from.
pub type CheckedSide = Option<(Side, SideOrigin)>;

/// Where a checked side came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideOrigin {
    Authored,
    Inferred,
}

/// What the gates print about the document: `dataflow` / `flows`.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    pub diagram: &'a str,
    pub collection: &'a str,
}

/// An edge label's rect and anchor (`labelRects[]`).
#[derive(Clone, Debug, PartialEq)]
pub struct LabelRect {
    /// The owning relation's index.
    pub rel: usize,
    pub text: String,
    pub rect: Rect,
    /// The text anchor (`lx`, `ly`), for the repair suggestions.
    pub anchor: Pt,
    /// Whether the relation authors `labelAt`; its finite `labelDx`/`labelDy` (else 0). The
    /// relative repair hint is built from these.
    pub authored_at: bool,
    pub authored_dx: f64,
    pub authored_dy: f64,
}

/// An opaque obstacle (a node) with its id.
#[derive(Clone, Debug, PartialEq)]
pub struct Obstacle {
    pub id: String,
    pub rect: Rect,
}

/// Recorded diagnostics and problems, in the order Archify produces them.
#[derive(Default, Debug)]
pub struct Sink {
    pub recorded: Vec<Diagnostic>,
    pub problems: Vec<String>,
    /// Rich diagnostics a renderer passes to `throwDiagnosticProblems` instead of recording them
    /// (`diagnostics.push`): they replace the plain `layout/constraint` of their problem, in the
    /// problem's place.
    pub details: Vec<Diagnostic>,
}

impl Sink {
    /// `recordDiagnostic` alone (a warning the renderer does not throw on).
    pub fn record(&mut self, d: Diagnostic) {
        self.recorded.push(d);
    }

    /// A rich diagnostic that is also a problem.
    pub fn error(&mut self, d: Diagnostic) {
        self.problems.push(d.message.clone());
        self.recorded.push(d);
    }

    /// A plain `validate*` sentence.
    pub fn problem(&mut self, message: String) {
        self.problems.push(message);
    }

    /// A problem with a rich diagnostic that is *not* recorded when found: it is reported in the
    /// problem's own position (`diagnostics.push` + `problems.push` in `validateArchitecture`).
    pub fn detailed(&mut self, d: Diagnostic) {
        self.problems.push(d.message.clone());
        self.details.push(d);
    }

    pub fn is_empty(&self) -> bool {
        self.problems.is_empty()
    }

    /// `throwDiagnosticProblems`: the recorded diagnostics, then every problem not already recorded
    /// as a `layout/constraint` carrying `plain` (the subject and, per problem, its `rule`).
    pub fn finish(self, plain: impl Fn(&str) -> Subject) -> Outcome {
        let mut all = self.recorded;
        for message in &self.problems {
            let detail = self.details.iter().find(|d| &d.message == message).cloned();
            all.push(detail.unwrap_or_else(|| {
                Diagnostic::error("layout/constraint", message).with_subject(plain(message))
            }));
        }
        Outcome { diagnostics: dedup_by_message(all), problems: self.problems }
    }
}

/// What a geometry validation found.
#[derive(Debug, Default)]
pub struct Outcome {
    /// As reported: recorded first, then the plain problems.
    pub diagnostics: Vec<Diagnostic>,
    /// The problem messages in validation order (the receipt's `error` bullets).
    pub problems: Vec<String>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// A JS number as `String(x)` prints it (whole numbers carry no `.0`, `-0` is `0`).
pub fn num(x: f64) -> String {
    if x == 0.0 { "0".to_owned() } else { format!("{x}") }
}

/// `Math.round(x * 10) / 10`.
pub fn round1(x: f64) -> f64 {
    js_round(x * 10.0) / 10.0
}

/// `p.map(v => Math.round(v * 10) / 10).join(', ')`.
pub fn point1(p: Pt) -> String {
    format!("{}, {}", num(round1(p[0])), num(round1(p[1])))
}

/// `p.map(Math.round).join(', ')`.
pub fn point0(p: Pt) -> String {
    format!("{}, {}", num(js_round(p[0])), num(js_round(p[1])))
}

/// `formatRect`: `[x, y, w, h]` rounded to integers.
pub fn format_rect(r: &Rect) -> String {
    format!("[{}, {}, {}, {}]", num(js_round(r.x)), num(js_round(r.y)), num(js_round(r.width)), num(js_round(r.height)))
}

/// A point as a JSON array in evidence.
pub fn pt_json(p: Pt) -> Value {
    Value::Array(vec![crate::diag::json_num(p[0]), crate::diag::json_num(p[1])])
}

/// A rect as evidence (`{x, y, width, height}`).
pub fn rect_json(r: &Rect) -> Value {
    let mut m = Map::new();
    for (k, v) in [("x", r.x), ("y", r.y), ("width", r.width), ("height", r.height)] {
        m.insert(k.to_owned(), crate::diag::json_num(v));
    }
    Value::Object(m)
}

/// An evidence map from key/value pairs.
pub fn evidence<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// `relationshipSubject`.
pub fn rel_subject(ctx: &Ctx<'_>, rel: &Rel) -> Subject {
    let mut s = Subject::of(ctx.diagram);
    s.collection = Some(ctx.collection.to_owned());
    s.index = Some(rel.index as i64);
    s.id = rel.id.clone();
    if !rel.from.is_empty() {
        s = s.with_extra("from", rel.from.clone().into());
    }
    if !rel.to.is_empty() {
        s = s.with_extra("to", rel.to.clone().into());
    }
    s
}

/// `relationshipSubject` as the nested `otherRelationship` evidence.
pub fn rel_subject_json(ctx: &Ctx<'_>, rel: &Rel) -> Value {
    serde_json::to_value(rel_subject(ctx, rel)).unwrap_or(Value::Null)
}

/// `rePlanHint(relations, fallback)`.
pub fn re_plan_hint(rels: &[&Rel], fallback: &str) -> String {
    let mut controls: Vec<&str> = Vec::new();
    for rel in rels {
        for key in PLAN_KEYS {
            if rel.controls.contains(&key) && !controls.contains(&key) {
                controls.push(key);
            }
        }
    }
    if controls.is_empty() {
        return fallback.to_owned();
    }
    format!(
        "if the authored {} are not required by the user, remove them so the renderer can re-plan; otherwise preserve that intent and {fallback}",
        controls.join("/")
    )
}

/// `${collection}[${index}]${ id "x"} "from" -> "to"`.
pub fn describe(ctx: &Ctx<'_>, rel: &Rel) -> String {
    let id = rel.id.as_deref().map(|i| format!(" id \"{i}\"")).unwrap_or_default();
    format!("{}[{}]{id} \"{}\" -> \"{}\"", ctx.collection, rel.index, rel.from, rel.to)
}
