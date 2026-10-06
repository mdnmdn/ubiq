//! What the interface does with `ubiq_archify::compile::compile` (P1.10): run it off the UI thread
//! on text the tab already holds, keep the parts a frame needs ([`Outcome`]), and turn a clicked
//! diagnostic into a [`Selection`].
//!
//! `compile` is pure (D3), so the call is allowed here; only the scheduling is GPUI. The viewer
//! calls [`drive`] from its draw with the tab's buffer text; when the text hashes differently from
//! the one last scheduled (an open, an edit, a reload from disk), `Ui` hands back a sequence number
//! and a delay. A task waits out the delay, compiles on the background executor, and hands the
//! result back; `Ui` drops it when a newer run has been scheduled since.
//!
//! The rest of the file is pure and unit-tested: the outcome of one run, the path-to-offset finder
//! over the buffer text, and the diagnostic-to-selection rule.

use std::sync::Arc;
use std::time::Duration;

use ubiq_archify::compile::{Opts, compile};
use ubiq_archify::diag::{Diagnostic, Severity};
use ubiq_archify::model::Doc;
use ubiq_archify::model::common::Animation;
use ubiq_archify::scene::{Bounds, GroupId, GroupKind, Scene};
use ubiq_archify::tokens::{Preset, restyle};
use gpui::{Context, WeakEntity};
use crate::app::AppState;

use crate::state::archify::ui;

/// How long an edit waits for a quieter moment before the document is compiled again.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// How soon a trigger runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// The first text of a tab, a changed override: at the next frame.
    Now,
    /// An edit: after [`DEBOUNCE`] with no further edit.
    Debounce,
}

/// What a run needs, copied out of the tab so the task owns it.
#[derive(Clone, Debug)]
pub struct Run {
    pub seq: u64,
    pub text: String,
    /// The file's relative path: `receipt.input`, and where the type comes from when the document
    /// does not say.
    pub rel: String,
    /// The settings' quality override (`Opts.quality`, D15).
    pub quality: Option<String>,
    /// The settings' palette: a preset name, or `document` for the one the file names (P9.1).
    pub palette: String,
}

/// One run's result, reduced to what the interface reads.
#[derive(Debug)]
pub struct Outcome {
    pub ok: bool,
    /// The reported profile: `advisory`, `standard` or `showcase`.
    pub profile: &'static str,
    /// The receipt's diagnostics, then the warnings of stages that passed.
    pub diagnostics: Vec<Diagnostic>,
    /// The scene of the layout, when V9 passed and the type has one.
    pub scene: Option<Arc<Scene>>,
    /// The document asks for the ambient trace (`meta.animation: "trace"`): what makes its motion
    /// governor capable (`animate.rs`, P7.3).
    pub trace: bool,
}

/// `meta.animation == "trace"` of a typed document.
fn traces(doc: &Doc) -> bool {
    let animation = match doc {
        Doc::Architecture(d) => d.meta.animation,
        Doc::Workflow(d) => d.meta.animation,
        Doc::Sequence(d) => d.meta.animation,
        Doc::Dataflow(d) => d.meta.animation,
        Doc::Lifecycle(d) => d.meta.animation,
    };
    animation == Some(Animation::Trace)
}

impl Outcome {
    pub fn errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Warning)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.diagnostics.len() - self.errors()
    }
}

/// Compile `run.text`. Pure; the caller puts it on the background executor.
pub fn run_compile(run: &Run) -> Outcome {
    let opts = Opts {
        input: &run.rel,
        doc_type: None,
        quality: run.quality.as_deref(),
    };
    let compiled = compile(&run.text, &opts);
    let mut diagnostics = compiled.receipt.diagnostics.clone();
    diagnostics.extend(compiled.warnings.iter().cloned());
    // The settings' palette over the preset the document names (P9.1, D38).
    let preset = Preset::pick(&run.palette, compiled.doc.as_ref().and_then(Doc::visual_preset));
    Outcome {
        ok: compiled.receipt.ok,
        profile: compiled.profile,
        diagnostics,
        // `Layout::scene` is the one arm every layout adds to.
        scene: compiled.layout.as_ref().map(|l| {
            let mut scene = l.scene().clone();
            restyle(&mut scene, preset);
            Arc::new(scene)
        }),
        trace: compiled.doc.as_ref().is_some_and(traces),
    }
}

/// Compile tab `key`, holding `text` of the file at `rel`, when its text is not the one the last
/// compile was scheduled for (`Ui::want_compile`). Cheap when nothing changed, so the viewer calls
/// it every frame.
pub fn drive(key: &str, rel: &str, text: &str, cx: &mut Context<AppState>) {
    let Some(job) = ui(cx).want_compile(key, text) else {
        return;
    };
    let (key, rel, text) = (key.to_string(), rel.to_string(), text.to_string());
    cx.spawn(async move |this: WeakEntity<AppState>, cx| {
        if job.due == Due::Debounce {
            cx.background_executor().timer(DEBOUNCE).await;
        }
        // Not current: the text changed again since, and that change has its own task.
        let Ok(true) = this.update(cx, |_, cx| ui(cx).is_current(&key, job.seq)) else {
            return;
        };
        let run = Run {
            seq: job.seq,
            text,
            rel,
            quality: job.quality,
            palette: job.palette,
        };
        let seq = run.seq;
        let outcome = cx
            .background_executor()
            .spawn(async move { run_compile(&run) })
            .await;
        let _ = this.update(cx, |_, cx| {
            if ui(cx).finish_compile(&key, seq, outcome) {
                cx.notify();
            }
        });
    })
    .detach();
}

// --------------------------------------------------------------------------------------------- //
// A clicked diagnostic
// --------------------------------------------------------------------------------------------- //

/// What a click on a diagnostic does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    /// The node to focus in the picture: the subject's node, or an edge's source node (the painter
    /// focuses nodes, and a focused node lights its edges).
    pub focus: Option<String>,
    /// The box the camera brings into view (P5.3): the bounds of the subject's group, which is the
    /// edge's own group when the diagnostic names an edge, not only the focused node.
    pub reveal: Option<Bounds>,
    /// Where to put the source cursor: 0-based line and UTF-16 column.
    pub cursor: Option<(u32, u32)>,
}

/// The selection of `d` over the picture `scene` (if any) and the buffer `text`.
pub fn select(d: &Diagnostic, scene: Option<&Scene>, text: &str) -> Selection {
    let target = scene.and_then(|scene| target_of(d, scene));
    Selection {
        reveal: target
            .as_ref()
            .and_then(|(_, group)| Some(scene?.group(*group)?.bounds)),
        focus: target.map(|(node, _)| node),
        cursor: cursor_of(d, text),
    }
}

/// The node `d` is about and the group the camera should reveal: its `id` or `identity` when that
/// is a node (its own group), else the source of the edge it names by edge id (the edge's group),
/// else the `from`/`to` of an edge subject (that node's group).
fn target_of(d: &Diagnostic, scene: &Scene) -> Option<(String, GroupId)> {
    let s = &d.subject;
    let named = [s.id.as_deref(), s.identity.as_deref()];
    let node_group = |id: &str| scene.node(id).map(|group| (id.to_string(), group));
    if let Some(found) = named.iter().flatten().find_map(|id| node_group(id)) {
        return Some(found);
    }
    let edge = (0..scene.groups.len() as u32).map(GroupId).find_map(|group| {
        let g = scene.group(group)?;
        let e = g.edge.as_ref().filter(|_| g.kind == GroupKind::Edge)?;
        let id = e.id.as_deref()?;
        named
            .iter()
            .flatten()
            .any(|n| *n == id)
            .then(|| (e.from.clone(), group))
    });
    if edge.is_some() {
        return edge;
    }
    ["from", "to"]
        .iter()
        .filter_map(|k| s.extra.get(*k)?.as_str())
        .find_map(node_group)
}

/// The cursor for `d`: its JSON path in `text`, else the line and column a parse error names.
fn cursor_of(d: &Diagnostic, text: &str) -> Option<(u32, u32)> {
    if let Some(path) = d.subject.path.as_deref()
        && let Some(offset) = path_offset(text, path)
    {
        return Some(line_col(text, offset));
    }
    if d.code == "input/json-parse" {
        return parse_error_position(&d.message);
    }
    None
}

/// `line 3 column 7` (1-based, serde_json's wording) as a 0-based position.
fn parse_error_position(message: &str) -> Option<(u32, u32)> {
    let number_after = |word: &str| -> Option<u32> {
        let rest = message.split(word).nth(1)?.trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let line = number_after("line ")?;
    let column = number_after("column ")?;
    Some((line.saturating_sub(1), column.saturating_sub(1)))
}

/// 0-based line and UTF-16 column of byte `offset` in `text`.
pub fn line_col(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let head = &text.as_bytes()[..offset];
    let line = head.iter().filter(|b| **b == b'\n').count();
    let start = head.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let column = text
        .get(start..offset)
        .map_or(offset - start, |s| s.encode_utf16().count());
    (line as u32, column as u32)
}

/// The byte offset of the JSON Pointer `pointer` in `text`: the start of the key when the last
/// segment names an object member, of the value when it indexes an array. When the path leaves the
/// document (a missing property, an index past the end), the deepest place that exists, so a
/// "required property is absent" diagnostic lands on the object that lacks it. `/` and the empty
/// pointer are the root. `None` when `text` has no value at all.
///
/// Not a validator: it walks as long as the text is well formed, and gives up where it is not.
pub fn path_offset(text: &str, pointer: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut at = skip_ws(b, 0);
    if at >= b.len() {
        return None;
    }
    // Archify writes the root as `/`.
    let segments: Vec<String> = if pointer.len() <= 1 {
        Vec::new()
    } else {
        pointer
            .split('/')
            .skip(1)
            .map(|s| s.replace("~1", "/").replace("~0", "~"))
            .collect()
    };
    for (n, segment) in segments.iter().enumerate() {
        match b.get(at) {
            Some(b'{') => match member(text, at, segment) {
                // The last segment answers with its key; the walk descends from the value.
                Some((key_at, _)) if n + 1 == segments.len() => return Some(key_at),
                Some((_, value_at)) => at = value_at,
                None => return Some(at),
            },
            Some(b'[') => match element(b, at, segment) {
                Some(value_at) => at = value_at,
                None => return Some(at),
            },
            _ => return Some(at),
        }
    }
    Some(at)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The end of the string whose opening quote is at `i`.
fn string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// The end of the value starting at `i` (which is not whitespace).
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = string_end(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']') && !b[j].is_ascii_whitespace()
            {
                j += 1;
            }
            Some(j)
        }
    }
}

/// In the object opening at `open`, the member named `name`: where its key and its value start.
fn member(text: &str, open: usize, name: &str) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let mut i = skip_ws(b, open + 1);
    while i < b.len() && b[i] == b'"' {
        let key_end = string_end(b, i)?;
        let key: String = serde_json::from_str(&text[i..key_end]).ok()?;
        let colon = skip_ws(b, key_end);
        if b.get(colon) != Some(&b':') {
            return None;
        }
        let value_at = skip_ws(b, colon + 1);
        if key == name {
            return Some((i, value_at));
        }
        let after = skip_ws(b, value_end(b, value_at)?);
        if b.get(after) != Some(&b',') {
            return None;
        }
        i = skip_ws(b, after + 1);
    }
    None
}

/// In the array opening at `open`, where element `index` starts.
fn element(b: &[u8], open: usize, index: &str) -> Option<usize> {
    let want: usize = index.parse().ok()?;
    let mut i = skip_ws(b, open + 1);
    for n in 0.. {
        if i >= b.len() || matches!(b[i], b']' | b',') {
            return None;
        }
        if n == want {
            return Some(i);
        }
        let after = skip_ws(b, value_end(b, i)?);
        if b.get(after) != Some(&b',') {
            return None;
        }
        i = skip_ws(b, after + 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::diag::Subject;
    use serde_json::Value;

    const EVENT_STREAM: &str =
        include_str!("../../../../ubiq-archify/tests/fixtures/event-stream.dataflow.golden.json");

    fn event_stream_doc() -> String {
        let golden: Value = serde_json::from_str(EVENT_STREAM).unwrap();
        golden["source_doc"].to_string()
    }

    fn run(text: &str, quality: Option<&str>) -> Outcome {
        run_compile(&Run {
            seq: 1,
            text: text.into(),
            rel: "x.dataflow.json".into(),
            quality: quality.map(str::to_string),
            palette: "document".into(),
        })
    }

    fn at(text: &str, pointer: &str) -> String {
        let offset = path_offset(text, pointer).unwrap();
        text[offset..].chars().take(8).collect()
    }

    const DOC: &str = "{\n  \"a\": 1,\n  \"nodes\": [\n    {\"id\": \"x\", \"k\": \"a\\\"b\"},\n    {\"id\": \"y\"}\n  ],\n  \"b/c\": {\"~d\": [true, null]}\n}";

    #[test]
    fn a_real_document_compiles_to_a_scene_and_a_bad_one_to_diagnostics() {
        let good = run(&event_stream_doc(), None);
        assert!(good.ok, "{:?}", good.diagnostics);
        assert!(good.scene.is_some(), "dataflow lays out");
        assert_eq!(good.errors(), 0);

        let bad = run("{\"diagram_type\":\"dataflow\"}", None);
        assert!(!bad.ok && bad.scene.is_none());
        assert!(bad.errors() > 0);
    }

    #[test]
    fn the_override_names_the_profile() {
        // The fixture authors its own profile; the override wins over it.
        let authored = run(&event_stream_doc(), None).profile;
        let other = if authored == "advisory" {
            "standard"
        } else {
            "advisory"
        };
        assert_eq!(run(&event_stream_doc(), Some(other)).profile, other);
    }

    #[test]
    fn a_path_lands_on_its_key_or_element() {
        assert_eq!(at(DOC, "/"), "{\n  \"a\":");
        assert_eq!(at(DOC, ""), "{\n  \"a\":");
        assert_eq!(at(DOC, "/a"), "\"a\": 1,\n");
        assert_eq!(at(DOC, "/nodes"), "\"nodes\":");
        assert_eq!(at(DOC, "/nodes/0"), "{\"id\": \"");
        assert_eq!(at(DOC, "/nodes/1/id"), "\"id\": \"y");
        assert_eq!(at(DOC, "/nodes/0/k"), "\"k\": \"a\\");
        assert_eq!(at(DOC, "/b~1c/~0d/1"), "null]}\n}");
    }

    #[test]
    fn a_path_that_leaves_the_document_stops_at_what_exists() {
        // A missing member: the object that lacks it.
        assert_eq!(at(DOC, "/nodes/1/label"), "{\"id\": \"");
        // An index past the end: the array.
        assert_eq!(at(DOC, "/nodes/9"), "[\n    {\"");
        assert_eq!(at(DOC, "/zzz/deeper"), "{\n  \"a\":");
        assert_eq!(path_offset("   ", "/a"), None);
        assert!(path_offset("{\"a\": ", "/a").is_some());
    }

    #[test]
    fn offsets_become_lines_and_utf16_columns() {
        let text = "ab\nc\u{e9}\u{1F600}d";
        assert_eq!(line_col(text, 0), (0, 0));
        assert_eq!(line_col(text, 3), (1, 0));
        let d = text.find('d').unwrap();
        // c, e-acute (1 unit each), the emoji (2 units).
        assert_eq!(line_col(text, d), (1, 4));
        assert_eq!(line_col(text, 999), line_col(text, text.len()));
    }

    fn about(f: impl FnOnce(Subject) -> Subject) -> Diagnostic {
        Diagnostic::error("graph/x", "m").with_subject(f(Subject::of("dataflow")))
    }

    #[test]
    fn a_node_subject_focuses_the_node_and_an_edge_subject_its_source() {
        let outcome = run(&event_stream_doc(), None);
        let scene = outcome.scene.unwrap();
        let node = scene
            .groups
            .iter()
            .find_map(|g| g.node_id.clone())
            .expect("a node");
        let edge = scene
            .groups
            .iter()
            .find_map(|g| g.edge.clone().filter(|_| g.kind == GroupKind::Edge))
            .expect("an edge");

        let by_identity = about(|s| s.with_identity(node.clone()));
        assert_eq!(
            select(&by_identity, Some(&scene), "{}").focus,
            Some(node.clone())
        );
        let node_bounds = scene.group(scene.node(&node).unwrap()).unwrap().bounds;
        assert_eq!(
            select(&by_identity, Some(&scene), "{}").reveal,
            Some(node_bounds),
            "the camera reveals the node's own group"
        );

        let mut by_id = about(|s| s);
        by_id.subject.id = Some(node.clone());
        assert_eq!(select(&by_id, Some(&scene), "{}").focus, Some(node.clone()));

        let by_ends = about(|s| s.with_extra("from", edge.from.clone().into()));
        assert_eq!(
            select(&by_ends, Some(&scene), "{}").focus,
            Some(edge.from.clone())
        );

        if let Some(edge_id) = edge.id.clone() {
            let mut by_edge = about(|s| s);
            by_edge.subject.id = Some(edge_id);
            let picked = select(&by_edge, Some(&scene), "{}");
            assert_eq!(picked.focus, Some(edge.from));
            let edge_group = scene
                .groups
                .iter()
                .find(|g| g.edge.as_ref().is_some_and(|e| e.id == edge.id))
                .unwrap();
            assert_eq!(
                picked.reveal,
                Some(edge_group.bounds),
                "an edge subject reveals the edge, not its source node"
            );
        }

        let unknown = about(|s| s.with_identity("no-such-node"));
        assert_eq!(select(&unknown, Some(&scene), "{}"), Selection::default());
        assert_eq!(
            select(&by_identity, None, "{}").focus,
            None,
            "no picture, no focus"
        );
    }

    #[test]
    fn without_a_node_the_cursor_follows_the_path_or_the_parse_error() {
        let by_path = about(|s| s.with_path("/nodes/1/id"));
        assert_eq!(select(&by_path, None, DOC).cursor, Some((4, 5)));

        let parse = Diagnostic::error(
            "input/json-parse",
            "Input JSON could not be parsed: expected value at line 3 column 7",
        );
        assert_eq!(select(&parse, None, "{}").cursor, Some((2, 6)));
        assert_eq!(select(&about(|s| s), None, DOC).cursor, None);
    }

    #[test]
    fn a_parse_error_of_the_real_pipeline_has_a_position() {
        let outcome = run("{\n  \"a\": ,\n}", None);
        let d = &outcome.diagnostics[0];
        assert_eq!(d.code, "input/json-parse");
        assert!(select(d, None, "").cursor.is_some());
    }
}
