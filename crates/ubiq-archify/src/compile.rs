//! `compile(text, opts) -> Compiled`: the staged pipeline V1-V10 (P1.5, P9.2), and [`peek`], the two
//! fields and the node count a document list shows without validating.
//!
//! # Stages (00 section 3.1)
//!
//! V1 parse, V2 `meta.output` syntax, V3 schema (all errors), then on a typed [`Doc`] V5
//! relationship ids, V6 engineering profile, V7 repository evidence (the header, then
//! `root-required`; with [`compile_with`] and the host's verifier, the whole verification against a
//! checkout, D39) and V9: for dataflow the whole of `validateDataflow` (graph
//! and geometry rules, `gates::dataflow`) and then the layout, whose [`Layout`] and composition
//! block land in the result (sequence likewise, `gates::sequence`, and lifecycle v2 on the grid
//! router, `gates::lifecycle`; architecture too: the router cascade and the whole of
//! `validateArchitecture`, `gates::architecture`; lifecycle v1 and v2 both go through `Plan`); for
//! workflow the semantic contract, then v1 on the fixed plan (`gates::workflow_v1`) or v2 on the
//! readable solver and router with `validateWorkflow` between routing and the canvas
//! (`gates::workflow_v2`; the v2 pre-routing graph checks run inside that compile). Then V10, the
//! artifact checker (`gates::artifact`) on the laid-out scene: its `checks` and `composition` are
//! the receipt's, and a document it rejects is `stage: "check"` with its layout kept. A
//! stage with an error stops the pipeline; a stage with only warnings (V4 `i18n/*`) does not.
//! `stages_run` names what ran, so a pass never claims more than it checked.
//!
//! # Quality (D15)
//!
//! The reported profile is `opts.quality`, else the document's `meta.quality_profile`, else
//! `standard` (what the real CLI reports; 00 section 3.3's "advisory" is a gate state, not a
//! reported one). The override changes gate severity only, and every implemented stage is
//! severity-independent, so today it only names the profile in the receipt.

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::diag::{Candidate, Diagnostic, Receipt, Stage, Subject, post_filter};
use crate::gates::{self, Gate, artifact, workflow_v1, workflow_v2};
use crate::layout::architecture_build::{self as architecture_layout, ArchitectureGeometry};
use crate::layout::dataflow::{self as dataflow_layout, DataflowGeometry};
use crate::layout::lifecycle::{self as lifecycle_layout, LifecycleGeometry};
use crate::layout::sequence::{self as sequence_layout, SequenceGeometry};
use crate::layout::workflow::legacy::{self as legacy_layout, LegacyGeometry, LegacyPlan};
use crate::layout::workflow::readable_build::ReadableGeometry;
use crate::model::Doc;
use crate::model::common::SchemaVersion;
use crate::scene::Scene;
use crate::schema::{self, DOC_TYPES, portable_path};
use crate::{engineering, evidence, graph};

/// What a list row needs from a document, without validating it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Peek {
    pub title: Option<String>,
    pub diagram_type: Option<String>,
    /// Components, nodes, participants or states: whichever collection the type has.
    pub nodes: Option<usize>,
}

/// `meta.title`, `diagram_type` and the node count of `text`, or the empty [`Peek`] when it is not
/// a JSON object. Never an error: a document that fails validation still has a row.
pub fn peek(text: &str) -> Peek {
    #[derive(Deserialize)]
    struct Meta {
        title: Option<String>,
    }
    #[derive(Deserialize)]
    struct Head {
        diagram_type: Option<String>,
        meta: Option<Meta>,
        components: Option<Vec<IgnoredAny>>,
        nodes: Option<Vec<IgnoredAny>>,
        participants: Option<Vec<IgnoredAny>>,
        states: Option<Vec<IgnoredAny>>,
    }
    match serde_json::from_str::<Head>(text) {
        Ok(head) => Peek {
            title: head.meta.and_then(|m| m.title),
            diagram_type: head.diagram_type,
            nodes: head
                .components
                .or(head.nodes)
                .or(head.participants)
                .or(head.states)
                .map(|v| v.len()),
        },
        Err(_) => Peek::default(),
    }
}

/// What the caller says about the document.
#[derive(Debug, Default, Clone)]
pub struct Opts<'a> {
    /// What `receipt.input` and the diagnostics' `subject.input` name.
    pub input: &'a str,
    /// The diagram type; absent, it comes from `diagram_type`, then from `<name>.<type>.json`.
    pub doc_type: Option<&'a str>,
    /// `advisory`, `standard` or `showcase`: the override of D15.
    pub quality: Option<&'a str>,
}

/// The geometry of a laid-out workflow: the `fixed-v1` plan or the `readable-v2` solution.
#[derive(Debug, Clone)]
pub enum WorkflowGeometry {
    V1(Box<LegacyGeometry>),
    V2(Box<ReadableGeometry>),
}

impl WorkflowGeometry {
    /// The final canvas.
    pub fn view_box(&self) -> [f64; 2] {
        match self {
            WorkflowGeometry::V1(g) => g.view_box,
            WorkflowGeometry::V2(g) => g.view_box,
        }
    }
}

/// The resolved layout: the scene to paint and the raw geometry the goldens and gates read.
/// All five types: architecture, dataflow, sequence, lifecycle (v1 and v2) and workflow (v1 and v2).
#[derive(Debug, Clone)]
pub enum Layout {
    Architecture {
        scene: Box<Scene>,
        geometry: Box<ArchitectureGeometry>,
    },
    Dataflow {
        scene: Box<Scene>,
        geometry: Box<DataflowGeometry>,
    },
    Sequence {
        scene: Box<Scene>,
        geometry: Box<SequenceGeometry>,
    },
    /// Schema v2 only (P4.2); v1 is P4.3.
    Lifecycle {
        scene: Box<Scene>,
        geometry: Box<LifecycleGeometry>,
    },
    Workflow {
        scene: Box<Scene>,
        geometry: Box<WorkflowGeometry>,
    },
}

impl Layout {
    /// The scene of any layout.
    pub fn scene(&self) -> &Scene {
        match self {
            Layout::Architecture { scene, .. }
            | Layout::Dataflow { scene, .. }
            | Layout::Sequence { scene, .. }
            | Layout::Lifecycle { scene, .. }
            | Layout::Workflow { scene, .. } => scene,
        }
    }
}

/// What an architecture run measured when it did not end in a [`Layout`]: the `--layout-json`
/// receipt of a rejected layout is still repair evidence (`render-architecture.mjs:975`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRun {
    pub geometry: Box<ArchitectureGeometry>,
    /// The gates found no problem: only the authored legend (a render-time error, never in
    /// `--layout-json`) failed the run.
    pub valid: bool,
}

/// What the workflow compiler said of a document that reached V9: `--layout-json`'s receipt
/// (`{contract, diagnostics}`), also when the layout was rejected.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRun {
    /// `fixed-v1` or `readable-v2`.
    pub contract: &'static str,
    /// What the compiler threw (one per problem, in problem order), or, for a clean layout, the
    /// warnings the v2 gates recorded (`workflowDiagnostics`).
    pub diagnostics: Vec<Diagnostic>,
}

/// The result of [`compile`].
#[derive(Debug)]
pub struct Compiled {
    /// Archify's `validate` receipt (00 section 3.6), already post-filtered.
    pub receipt: Receipt,
    /// The typed document, once V3 passed (also when a later stage failed).
    pub doc: Option<Doc>,
    /// The stages of 00 section 3.1 that ran, in order.
    pub stages_run: Vec<&'static str>,
    /// Warnings of stages that passed; Archify prints these to stderr, never in the receipt.
    pub warnings: Vec<Diagnostic>,
    /// The reported quality profile: `advisory`, `standard` or `showcase`.
    pub profile: &'static str,
    /// The layout, once V9 passed (all five types). Kept when V10 rejects the document, so the
    /// painter can show the picture beside the checker's diagnostics.
    pub layout: Option<Layout>,
    /// An architecture run that reached the layout but failed V9 (see [`ArchitectureRun`]).
    pub architecture: Option<ArchitectureRun>,
    /// A workflow run that reached V9, laid out or not (see [`WorkflowRun`]).
    pub workflow: Option<WorkflowRun>,
}

impl Compiled {
    pub fn ok(&self) -> bool {
        self.receipt.ok
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.receipt.diagnostics
    }
}

/// The quality values `--quality` / the `quality` argument accept.
pub const QUALITIES: &[&str] = &["advisory", "standard", "showcase"];

fn canonical_profile(name: &str) -> Option<&'static str> {
    QUALITIES.iter().copied().find(|q| *q == name)
}

/// The default file extension (D30): the same JSON document, its type in `diagram_type`.
pub const EXTENSION: &str = ".archify";

/// Whether `name` is a `.archify` file (case-sensitive, like the `*.<type>.json` suffixes).
pub fn is_archify(name: &str) -> bool {
    name.ends_with(EXTENSION)
}

/// `a/b.dataflow.json` to `dataflow`. A `.archify` name carries no type (it is the document's
/// `diagram_type`), so it gives `None` (D30).
pub fn type_from_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".json")?;
    let kind = stem.rsplit('.').next()?;
    DOC_TYPES.contains(&kind).then(|| kind.to_owned())
}

/// `input/read`: a file that could not be read, as a failing receipt (stage `input`).
pub fn read_failure(doc_type: Option<&str>, input: &str, reason: &str) -> Receipt {
    let diagnostic = Diagnostic::error("input/read", &format!("Input could not be read: {reason}"))
        .with_subject(Subject::default().with_extra("input", input.into()))
        .with_fixes(["check the file exists, is inside the project and is UTF-8 text"]);
    Receipt::failure(
        "validate",
        Stage::Input,
        doc_type.unwrap_or("unknown"),
        input,
        reason,
        vec![diagnostic],
    )
}

/// A usage error as a receipt (`stage: arguments`, exit 2).
pub fn usage_failure(
    doc_type: Option<&str>,
    input: &str,
    code: &str,
    message: &str,
    fixes: &[String],
) -> Receipt {
    Receipt::failure(
        "validate",
        Stage::Arguments,
        doc_type.unwrap_or("unknown"),
        input,
        message,
        vec![Diagnostic::error(code, message).with_fixes(fixes)],
    )
}

struct Run<'a> {
    input: &'a str,
    /// The reported profile (`standard` when nothing says otherwise).
    profile: &'static str,
    /// The profile the gates see: the override, else the authored one, else none.
    gate: Option<&'static str>,
    stages: Vec<&'static str>,
}

impl Run<'_> {
    fn fail(
        self,
        stage: Stage,
        doc_type: &str,
        error: &str,
        diagnostics: Vec<Diagnostic>,
        doc: Option<Doc>,
        warnings: Vec<Diagnostic>,
    ) -> Compiled {
        Compiled {
            receipt: Receipt::failure("validate", stage, doc_type, self.input, error, diagnostics),
            doc,
            stages_run: self.stages,
            warnings,
            profile: self.profile,
            layout: None,
            architecture: None,
            workflow: None,
        }
    }
}

/// Verifies a document's repository evidence against a checkout (`--repo-root`): the whole of
/// `verifyRepositoryEvidence`, header and per-source shape included, or `None` when every citation
/// holds. The host builds it (the host's verifier); `compile` only calls it.
pub type EvidenceCheck<'a> = &'a dyn Fn(&Doc) -> Option<Diagnostic>;

/// Runs V1-V10 over `text`, evidence unverified: a document that declares source evidence is
/// refused with `repository-evidence/root-required`, as Archify refuses it without `--repo-root`.
/// See the module doc for the stage order and what each can report.
pub fn compile(text: &str, opts: &Opts<'_>) -> Compiled {
    compile_with(text, opts, None)
}

/// [`compile`] with an evidence verifier (V7): the host's answer to `--repo-root`. `compile` stays
/// pure; the closure is where `git` runs.
pub fn compile_with(text: &str, opts: &Opts<'_>, verify: Option<EvidenceCheck<'_>>) -> Compiled {
    let input = opts.input;
    let override_profile = opts.quality.and_then(canonical_profile);
    let mut run = Run {
        input,
        profile: override_profile.unwrap_or("standard"),
        gate: override_profile,
        stages: Vec::new(),
    };

    if let Some(q) = opts.quality
        && override_profile.is_none()
    {
        let message = format!("--quality must be advisory, standard or showcase, got \"{q}\"");
        let diagnostics = vec![Diagnostic::error("cli/invalid-option-value", &message)];
        let doc_type = opts.doc_type.unwrap_or("unknown");
        return run.fail(
            Stage::Arguments,
            doc_type,
            &message,
            diagnostics,
            None,
            vec![],
        );
    }

    // V1.
    run.stages.push("V1");
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => {
            let reason = e.to_string();
            let diagnostic = Diagnostic::error(
                "input/json-parse",
                &format!("Input JSON could not be parsed: {reason}"),
            )
            .with_subject(Subject::default().with_extra("input", input.into()))
            .with_evidence(Map::from_iter([(
                "reason".to_owned(),
                reason.clone().into(),
            )]))
            .with_fixes(["repair the JSON syntax and run validation again"]);
            let doc_type = opts.doc_type.unwrap_or("unknown");
            return run.fail(
                Stage::Input,
                doc_type,
                &reason,
                vec![diagnostic],
                None,
                vec![],
            );
        }
    };

    if override_profile.is_none()
        && let Some(authored) = value
            .pointer("/meta/quality_profile")
            .and_then(Value::as_str)
            .and_then(canonical_profile)
    {
        run.profile = authored;
        run.gate = Some(authored);
    }

    let doc_type = opts
        .doc_type
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("diagram_type")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| type_from_name(input));
    let Some(doc_type) = doc_type else {
        let message = if is_archify(input) {
            "a .archify file takes its type from diagram_type: set it in the document to one of the diagram types"
        } else {
            "the diagram type is unknown: pass type or set diagram_type"
        };
        let diagnostic = Diagnostic::error("cli/missing-option-value", message)
            .with_fixes([format!("use one of {}", DOC_TYPES.join(", "))]);
        return run.fail(
            Stage::Arguments,
            "unknown",
            message,
            vec![diagnostic],
            None,
            vec![],
        );
    };
    if !DOC_TYPES.contains(&doc_type.as_str()) {
        let diagnostics = schema::validate(&doc_type, &value);
        let error = diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_default();
        return run.fail(
            Stage::Arguments,
            &doc_type,
            &error,
            diagnostics,
            None,
            vec![],
        );
    }

    // V2.
    run.stages.push("V2");
    if let Some(output) = value.pointer("/meta/output").and_then(Value::as_str)
        && let Some(diagnostic) = portable_path::validate_authored_output(output)
    {
        let error = diagnostic.message.clone();
        return run.fail(
            Stage::Render,
            &doc_type,
            &error,
            vec![diagnostic],
            None,
            vec![],
        );
    }

    // V3.
    run.stages.push("V3");
    let diagnostics = schema::validate(&doc_type, &value);
    if !diagnostics.is_empty() {
        let lines: Vec<String> = diagnostics
            .iter()
            .map(|d| format!("  {}", d.message))
            .collect();
        let error = format!("{doc_type} schema validation failed:\n{}", lines.join("\n"));
        return run.fail(Stage::Render, &doc_type, &error, diagnostics, None, vec![]);
    }
    let doc = match Doc::from_value(&value) {
        Ok(doc) => doc,
        Err(reason) => {
            // The schema passed, so the model should read it: say so instead of a stack trace.
            let diagnostic = Diagnostic::error(
                crate::diag::UNCLASSIFIED,
                &format!("The document passed the schema but could not be read: {reason}"),
            );
            return run.fail(
                Stage::Render,
                &doc_type,
                &reason,
                vec![diagnostic],
                None,
                vec![],
            );
        }
    };

    // V5, V6, V7, then V9.
    type Step<'a> = (&'static str, &'a dyn Fn(&Doc, &Value) -> Vec<Diagnostic>);
    let steps: [Step<'_>; 5] = [
        // V4 (`i18n/*`, warnings only) and V8 (`brand/*`, the built-in marks; D37).
        ("V4", &|d, _| crate::i18n::check_doc(d)),
        ("V5", &|d, _| graph::relationship_ids(d)),
        ("V6", &engineering::check),
        ("V7", &|d, _| match verify {
            Some(check) => check(d).into_iter().collect(),
            None => evidence::check_unverified(d),
        }),
        ("V8", &|d, _| crate::brand::check(d)),
    ];
    let mut warnings = Vec::new();
    for (name, step) in steps {
        run.stages.push(name);
        let found = post_filter(step(&doc, &value));
        if found.iter().any(Diagnostic::is_error) {
            let error = stage_error(name, &doc_type, &found);
            return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
        }
        warnings.extend(found);
    }

    // V9. Every type runs the whole of its `validate*` (graph and geometry rules together, as
    // Archify does), then lays out.
    run.stages.push("V9");
    let mut layout = None;
    // What the artifact checker reads of the type's gate review (V10, below).
    let mut parts: Option<artifact::Parts> = None;
    let mut workflow = None;
    if let Doc::Sequence(s) = &doc {
        let (outcome, review) = gates::sequence::validate(s, Gate(run.gate));
        let found = post_filter(outcome.diagnostics);
        if !outcome.problems.is_empty() {
            let bullets: Vec<String> = outcome.problems.iter().map(|p| format!("- {p}")).collect();
            let error = format!("Sequence layout validation failed:\n{}", bullets.join("\n"));
            return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
        }
        warnings.extend(found);
        match sequence_layout::build(s) {
            Ok((scene, geometry)) => {
                parts = Some(artifact::sequence(s, &review, geometry.column_fit));
                layout = Some(Layout::Sequence { scene: Box::new(scene), geometry: Box::new(geometry) });
            }
            Err(found) => {
                let found = post_filter(found);
                let error = stage_error("V9", &doc_type, &found);
                return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            }
        }
    } else if let Doc::Lifecycle(l) = &doc
        && let Ok(plan) = lifecycle_layout::Plan::new(l)
    {
        // v1 and v2 (the grid router, or the architecture router for a v2 pin the grid plan
        // contradicts): the whole of `validateLifecycle`, then the layout.
        let (outcome, review) = gates::lifecycle::validate(&plan, Gate(run.gate));
        let found = post_filter(outcome.diagnostics);
        if !outcome.problems.is_empty() {
            let bullets: Vec<String> = outcome.problems.iter().map(|p| format!("- {p}")).collect();
            let error = format!("Lifecycle layout validation failed:\n{}", bullets.join("\n"));
            return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
        }
        warnings.extend(found);
        match lifecycle_layout::build(l) {
            Ok((scene, geometry)) => {
                layout = Some(Layout::Lifecycle { scene: Box::new(scene), geometry: Box::new(geometry) });
            }
            Err(found) => {
                let found = post_filter(found);
                let error = stage_error("V9", &doc_type, &found);
                return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            }
        }
        parts = Some(artifact::lifecycle(l, &review));
    } else if let Doc::Dataflow(d) = &doc {
        let (outcome, review) = gates::dataflow::validate(d, Gate(run.gate));
        let found = post_filter(outcome.diagnostics);
        if !outcome.problems.is_empty() {
            let bullets: Vec<String> = outcome.problems.iter().map(|p| format!("- {p}")).collect();
            let error = format!("Data-flow layout validation failed:\n{}", bullets.join("\n"));
            return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
        }
        warnings.extend(found);
        match dataflow_layout::build(d) {
            Ok((scene, geometry)) => {
                layout = Some(Layout::Dataflow { scene: Box::new(scene), geometry: Box::new(geometry) });
            }
            Err(found) => {
                let found = post_filter(found);
                let error = stage_error("V9", &doc_type, &found);
                return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            }
        }
        parts = Some(artifact::dataflow(d, &review));
    } else if let Doc::Architecture(a) = &doc {
        // A component with no placement that a connection uses crashes Archify's router
        // (`internal/unclassified`); nothing else is reported then.
        let crash: Vec<Diagnostic> = graph::layout_rules(&doc, &value)
            .into_iter()
            .filter(|d| d.code == crate::diag::UNCLASSIFIED)
            .collect();
        if !crash.is_empty() {
            let found = post_filter(crash);
            let error = stage_error("V9", &doc_type, &found);
            return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
        }
        // The whole of `validateArchitecture` (graph and geometry rules) on the routed layout; a
        // component with no finite position is measured without routes and reported by the gate.
        let built = architecture_layout::build(a);
        let geometry = match &built {
            Ok((_, geometry)) => geometry.clone(),
            Err(_) => ArchitectureGeometry::unrouted(a),
        };
        let (outcome, review) = gates::architecture::validate(a, &geometry, Gate(run.gate));
        let found = post_filter(outcome.diagnostics);
        if !outcome.problems.is_empty() {
            let bullets: Vec<String> = outcome.problems.iter().map(|p| format!("- {p}")).collect();
            let error = format!("Architecture layout validation failed:\n{}", bullets.join("\n"));
            let mut failed = run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            failed.architecture = Some(ArchitectureRun { geometry: Box::new(geometry), valid: false });
            return failed;
        }
        warnings.extend(found);
        let scene = match built {
            Ok((scene, _)) => scene,
            Err(found) => {
                let found = post_filter(found);
                let error = stage_error("V9", &doc_type, &found);
                return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            }
        };
        // The authored legend is only checked when the SVG is drawn, after the layout validates.
        if let Some(problem) = geometry.legend_problem.clone() {
            let found = post_filter(vec![problem]);
            let error = stage_error("V9", &doc_type, &found);
            let mut failed = run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            failed.architecture = Some(ArchitectureRun { geometry: Box::new(geometry), valid: true });
            return failed;
        }
        parts = Some(artifact::architecture(a, &review));
        layout = Some(Layout::Architecture { scene: Box::new(scene), geometry: Box::new(geometry) });
    } else if let Doc::Workflow(w) = &doc {
        // The semantic contract, then (v2) the first pre-routing failure; then the layout with the
        // whole of `validateWorkflow`: v1 on the fixed plan, v2 between routing and the canvas.
        let v2 = w.schema_version == SchemaVersion::V2;
        let contract = if v2 { "readable-v2" } else { "fixed-v1" };
        let gate = Gate(run.gate);
        // v2 runs the prologue inside its compile (with verified fixes); v1 has the semantic part only.
        let early = if v2 { Vec::new() } else { graph::workflow_prologue(w, None) };
        if !early.is_empty() {
            let found = post_filter(early.clone());
            let error = stage_error("V9", &doc_type, &found);
            let mut failed = run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
            failed.workflow = Some(WorkflowRun { contract, diagnostics: early });
            return failed;
        }
        if v2 {
            match workflow_v2::build(w, gate) {
                Ok((scene, geometry)) => {
                    parts = Some(artifact::workflow(&workflow_v2::review_of(&geometry), Some(&geometry)));
                    workflow = Some(WorkflowRun { contract, diagnostics: geometry.diagnostics.clone() });
                    layout = Some(Layout::Workflow {
                        scene: Box::new(scene),
                        geometry: Box::new(WorkflowGeometry::V2(Box::new(geometry))),
                    });
                }
                Err(failure) => {
                    let found = post_filter(failure.reported().to_vec());
                    let mut failed = run.fail(Stage::Render, &doc_type, &failure.error, found, Some(doc), warnings);
                    failed.workflow = Some(WorkflowRun { contract, diagnostics: failure.diagnostics });
                    return failed;
                }
            }
        } else {
            let plan = LegacyPlan::new(w);
            let (outcome, review) = workflow_v1::validate_v1(&plan, gate);
            let found = post_filter(outcome.diagnostics);
            if !outcome.problems.is_empty() {
                let bullets: Vec<String> = outcome.problems.iter().map(|p| format!("- {p}")).collect();
                let error = review
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("Workflow layout validation failed:\n{}", bullets.join("\n")));
                let mut failed = run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
                failed.workflow = Some(WorkflowRun { contract, diagnostics: review.compiler });
                return failed;
            }
            warnings.extend(found);
            match legacy_layout::build_plan(&plan) {
                Ok((scene, geometry)) => {
                    parts = Some(artifact::workflow(&review, None));
                    workflow = Some(WorkflowRun { contract, diagnostics: Vec::new() });
                    layout = Some(Layout::Workflow {
                        scene: Box::new(scene),
                        geometry: Box::new(WorkflowGeometry::V1(Box::new(geometry))),
                    });
                }
                Err(thrown) => {
                    // An authored legend that does not fit is thrown while the SVG is drawn, inside
                    // the compiler: it is the layout receipt's failure too.
                    let found = post_filter(thrown.clone());
                    let error = stage_error("V9", &doc_type, &found);
                    let mut failed = run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
                    failed.workflow = Some(WorkflowRun { contract, diagnostics: thrown });
                    return failed;
                }
            }
        }
    } else {
        let found = post_filter(graph::layout_rules(&doc, &value));
        if found.iter().any(Diagnostic::is_error) {
            let error = stage_error("V9", &doc_type, &found);
            return run.fail(Stage::Render, &doc_type, &error, found, Some(doc), warnings);
        }
        warnings.extend(found);
    }

    // V10: the artifact checker (`check-render-output.mjs`) on the laid-out scene. A document it
    // rejects stays laid out: the layout receipt does not run it, and the painter can show why.
    let mut checks = Vec::new();
    let mut composition = None;
    let report = match (&layout, &parts) {
        (Some(layout), Some(parts)) => {
            run.stages.push("V10");
            Some(artifact::check(&parts.input(&doc_type, layout.scene(), Gate(run.gate))))
        }
        _ => None,
    };
    if let Some(report) = report {
        if !report.ok {
            let checker = artifact::receipt(&report, input);
            let mut failed =
                run.fail(Stage::Check, &doc_type, "Final artifact check failed.", report.diagnostics, Some(doc), warnings);
            failed.receipt.checker = Some(checker);
            failed.layout = layout;
            failed.workflow = workflow;
            return failed;
        }
        checks = report.checks;
        composition = Some(report.composition);
    }

    let mut receipt = Receipt::success(
        "validate",
        &doc_type,
        input,
        Candidate {
            path: input.to_owned(),
            sha256: Sha256::digest(text.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            bytes: text.len() as u64,
        },
    );
    receipt.checks = checks;
    receipt.composition = Some(composition.unwrap_or_else(|| {
        json!({
            "schemaVersion": 1,
            "profile": run.profile,
            "status": "not-run",
            "summary": {"errors": 0, "warnings": 0},
        })
    }));
    Compiled {
        receipt,
        doc: Some(doc),
        stages_run: run.stages,
        warnings,
        profile: run.profile,
        layout,
        architecture: None,
        workflow,
    }
}

/// The receipt's `error` for a failing graph stage, in Archify's wording: the stage's header and
/// one `- message` line each, or the message alone when a single specific diagnostic threw it.
fn stage_error(stage: &str, doc_type: &str, diagnostics: &[Diagnostic]) -> String {
    let bullets = |header: &str| {
        let lines: Vec<String> = diagnostics
            .iter()
            .map(|d| format!("- {}", d.message))
            .collect();
        format!("{header}\n{}", lines.join("\n"))
    };
    match stage {
        "V5" => bullets("Relationship identity validation failed:"),
        "V6" => bullets("Engineering profile \"deployment-ownership\" failed:"),
        "V8" => crate::brand::error_text(diagnostics),
        "V9" if diagnostics.iter().all(|d| d.code == "layout/constraint") => {
            let label = match doc_type {
                "dataflow" => "Data-flow".to_owned(),
                other => {
                    let mut chars = other.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
            };
            bullets(&format!("{label} layout validation failed:"))
        }
        _ => diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataflow() -> Value {
        json!({"schema_version": 1, "diagram_type": "dataflow",
            "meta": {"title": "t", "output": "a.html"},
            "stages": [{"label": "In"}, {"label": "Out"}],
            "nodes": [
                {"id": "a", "type": "backend", "label": "A", "stage": 0, "row": 0},
                {"id": "b", "type": "backend", "label": "B", "stage": 1, "row": 0}],
            "flows": [{"from": "a", "to": "b", "label": "go"}]})
    }

    fn run(value: &Value, quality: Option<&str>) -> Compiled {
        compile(
            &value.to_string(),
            &Opts {
                input: "x.dataflow.json",
                quality,
                ..Opts::default()
            },
        )
    }

    #[test]
    fn peek_reads_title_type_and_node_count() {
        let p = peek(r#"{"diagram_type":"lifecycle","meta":{"title":"T"},"lanes":[]}"#);
        assert_eq!(p.title.as_deref(), Some("T"));
        assert_eq!(p.diagram_type.as_deref(), Some("lifecycle"));
        assert_eq!(p.nodes, None);
        assert_eq!(peek("not json"), Peek::default());
        assert_eq!(peek("{}"), Peek::default());
        assert_eq!(peek(&dataflow().to_string()).nodes, Some(2));
        assert_eq!(peek(r#"{"states":[{},{},{}]}"#).nodes, Some(3));
    }

    #[test]
    fn a_good_document_runs_every_stage_and_reports_the_profile() {
        let c = run(&dataflow(), None);
        assert!(c.ok(), "{:?}", c.diagnostics());
        assert_eq!(c.stages_run, ["V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9", "V10"]);
        assert!(c.doc.is_some());
        let Some(Layout::Dataflow { scene, geometry }) = &c.layout else {
            panic!("a dataflow compiles to a layout")
        };
        assert_eq!(scene.view_box, [940.0, 720.0]);
        assert_eq!(geometry.flows.len(), 1);
        assert_eq!(c.profile, "standard");
        let composition = c.receipt.composition.as_ref().unwrap();
        assert_eq!(composition["profile"], "standard");
        assert_eq!(composition["status"], "pass");
        assert_eq!(run(&dataflow(), Some("showcase")).profile, "showcase");
        let mut authored = dataflow();
        authored["meta"]["quality_profile"] = json!("showcase");
        assert_eq!(run(&authored, None).profile, "showcase");
        assert_eq!(run(&authored, Some("standard")).profile, "standard");
    }

    #[test]
    fn a_failing_stage_stops_the_pipeline() {
        let mut bad = dataflow();
        bad["nodes"][1]["stage"] = json!(9);
        let c = run(&bad, None);
        assert!(!c.ok());
        assert_eq!(c.stages_run.last(), Some(&"V9"));
        assert!(c.doc.is_some());
        assert!(
            c.receipt
                .error
                .as_deref()
                .unwrap()
                .starts_with("Data-flow layout validation failed:\n- ")
        );

        // V3 wins over a graph problem, and reports every schema error.
        bad["nodes"][0]["colour"] = json!("red");
        bad["nodes"][1]["type"] = json!("toaster");
        let c = run(&bad, None);
        assert_eq!(c.stages_run, ["V1", "V2", "V3"]);
        assert!(c.doc.is_none());
        assert_eq!(c.diagnostics().len(), 2);
    }

    #[test]
    fn type_comes_from_the_name_when_nothing_else_says() {
        assert_eq!(
            type_from_name("x/a.workflow.json").as_deref(),
            Some("workflow")
        );
        assert_eq!(type_from_name("a.json"), None);
        assert_eq!(type_from_name("<document>"), None);
        assert_eq!(type_from_name("a.archify"), None);
        assert!(is_archify("d/a.archify") && !is_archify("a.dataflow.json"));
    }

    #[test]
    fn an_archify_file_takes_its_type_from_diagram_type() {
        let at = |text: &str, doc_type: Option<&str>| {
            compile(text, &Opts { input: "d/a.archify", doc_type, quality: None })
        };
        let good = dataflow();
        let c = at(&good.to_string(), None);
        assert!(c.ok(), "{:?}", c.diagnostics());
        assert_eq!(c.receipt.doc_type, "dataflow");

        // An explicit type still overrides: the document then fails that schema.
        let c = at(&good.to_string(), Some("workflow"));
        assert!(!c.ok());
        assert_eq!(c.receipt.doc_type, "workflow");

        // Missing: a usage diagnostic that names the way out.
        let mut no_type = good.clone();
        no_type.as_object_mut().unwrap().remove("diagram_type");
        let c = at(&no_type.to_string(), None);
        assert_eq!(c.receipt.exit_code(), 2);
        assert_eq!(c.diagnostics()[0].code, "cli/missing-option-value");
        assert!(c.diagnostics()[0].message.contains("diagram_type"));
        assert!(c.diagnostics()[0].message.contains(".archify"));
        // With the type given as an argument it gets past the usage check; the schema still
        // requires the field.
        let c = at(&no_type.to_string(), Some("dataflow"));
        assert!(!c.ok() && c.diagnostics()[0].code.starts_with("schema/"), "{:?}", c.diagnostics());

        // Unknown: the unknown-type diagnostic, listing the valid ones.
        let mut unknown = good;
        unknown["diagram_type"] = json!("gantt");
        let c = at(&unknown.to_string(), None);
        assert_eq!(c.receipt.exit_code(), 2);
        assert_eq!(c.diagnostics()[0].code, "cli/unknown-diagram-type");
    }

    #[test]
    fn usage_and_input_failures() {
        let opts = Opts::default();
        let c = compile("{", &opts);
        assert_eq!(c.receipt.exit_code(), 1);
        assert_eq!(c.stages_run, ["V1"]);
        let c = compile("{}", &opts);
        assert_eq!(c.receipt.exit_code(), 2);
        let c = compile(
            &dataflow().to_string(),
            &Opts {
                quality: Some("gold"),
                ..opts
            },
        );
        assert_eq!(c.diagnostics()[0].code, "cli/invalid-option-value");
        assert!(c.stages_run.is_empty());
    }
}
