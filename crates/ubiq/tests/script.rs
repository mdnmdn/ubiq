//! The script facade, driven the way the sink's page drives it: a [`Run`] in, an outcome out.
//! Every case here is one the page can produce by hand, including the two that must not take the
//! process with them — a throw and an endless loop — the host surface a script reads and writes
//! through, and the two that never run anything: compiling TypeScript and checking syntax.

use ubiq::state::script::{
    self, ConsoleLevel, Dialect, EsTarget, HostFacts, OxcOptions, Run, ScriptEvent,
};

/// One program in the default dialect, with no facts behind it — what most cases here want.
fn run(prelude: &str, source: &str) -> script::ScriptOutcome {
    script::eval(Run::new(prelude, source))
}

/// The same, in TypeScript.
fn run_ts(prelude: &str, source: &str) -> script::ScriptOutcome {
    script::eval(Run::new(prelude, source).dialect(Dialect::Ts))
}

#[test]
fn an_interpreter_is_compiled_in() {
    assert!(script::available());
    assert_eq!(script::engine_name(), "QuickJS");
}

#[test]
fn a_value_comes_back() {
    let outcome = run("", "1 + 1");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.value.as_deref(), Some("2"));
}

#[test]
fn an_undefined_completion_is_no_value() {
    let outcome = run("", "let unused = 3;");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.value, None);
}

#[test]
fn console_levels_land_in_order() {
    let outcome = run(
        "",
        r#"console.log("one", 2);
           console.info("two");
           console.warn("three");
           console.error("four");"#,
    );

    assert_eq!(outcome.error, None);
    let levels: Vec<_> = outcome.lines.iter().map(|line| line.level).collect();
    assert_eq!(
        levels,
        vec![
            ConsoleLevel::Log,
            ConsoleLevel::Info,
            ConsoleLevel::Warn,
            ConsoleLevel::Error
        ]
    );
    assert_eq!(outcome.lines[0].text, "one 2");
    assert_eq!(outcome.lines[3].text, "four");
}

#[test]
fn an_object_prints_as_json() {
    let outcome = run("", r#"console.log({ a: 1, b: [true, null] }); 0"#);
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.lines[0].text, r#"{"a":1,"b":[true,null]}"#);
}

#[test]
fn a_cycle_does_not_become_the_runs_error() {
    let outcome = run("", "const a = {}; a.self = a; console.log(a); 7");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.lines[0].text, "[object]");
    assert_eq!(outcome.value.as_deref(), Some("7"));
}

#[test]
fn a_throw_becomes_an_error_and_does_not_panic() {
    let outcome = run("", r#"throw new Error("boom")"#);
    assert_eq!(outcome.value, None);
    let error = outcome.error.expect("a throw is reported as an error");
    assert!(error.contains("boom"), "{error}");
}

#[test]
fn a_syntax_error_is_reported_too() {
    let outcome = run("", "function (");
    assert!(outcome.error.is_some());
    assert_eq!(outcome.value, None);
}

/// The interrupt handler is the whole of the defence against a script that never yields. It is
/// checked against a short block limit here; the page's real limit is five seconds, and a test
/// that obeyed it would cost five seconds.
#[test]
fn an_endless_loop_hits_the_block_limit() {
    let outcome = script::eval_timed(Run::new("", "while (true) {}"), 200, 2_000);
    let error = outcome.error.expect("a stopped script reports an error");
    assert!(error.contains("block limit"), "{error}");
    assert!(
        outcome.elapsed_ms < 2_000,
        "the interrupt handler did not stop it: {}ms",
        outcome.elapsed_ms
    );
}

/// A run whose interpreter does not answer — the interrupt could not have fired, or the worker
/// died — is abandoned after the total budget. The choice to wait until the caller's deadline
/// and answer anyway is what keeps the page from hanging on a pathological native hang.
#[test]
fn a_run_past_the_total_limit_is_abandoned() {
    let outcome = script::eval_timed(Run::new("", "while (true) {}"), 10_000, 150);
    let error = outcome.error.expect("a timed-out run reports an error");
    assert!(error.contains("total time limit"), "{error}");
    assert!(
        outcome.elapsed_ms < 1_000,
        "the caller did not get its answer on time: {}ms",
        outcome.elapsed_ms
    );
}

#[test]
fn the_prelude_is_visible_to_the_source() {
    let outcome = run("function double(n) { return n * 2; }", "double(21)");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.value.as_deref(), Some("42"));
}

#[test]
fn a_prelude_that_throws_stops_the_run() {
    let outcome = run(
        r#"throw new Error("prelude broke")"#,
        r#"console.log("ran")"#,
    );
    let error = outcome.error.expect("the prelude's throw is the error");
    assert!(error.contains("prelude broke"), "{error}");
    assert!(outcome.lines.is_empty(), "the source must not have run");
}

#[test]
fn nothing_survives_between_runs() {
    let first = run("", "globalThis.left = 1; 1");
    assert_eq!(first.error, None);
    let second = run("", "typeof globalThis.left");
    assert_eq!(second.value.as_deref(), Some("undefined"));
}

#[test]
fn the_ubiq_global_answers() {
    let outcome = run(
        "",
        r#"[ubiq.version(), ubiq.platform(), ubiq.echo(12)].join("|")"#,
    );
    assert_eq!(outcome.error, None);
    let value = outcome.value.expect("the bindings return");
    let parts: Vec<_> = value.split('|').collect();
    assert_eq!(parts[0], ubiq::version::FULL);
    assert_eq!(parts[1], std::env::consts::OS);
    assert_eq!(parts[2], "12");
}

#[test]
fn ubiq_now_is_a_timestamp_and_ubiq_log_is_a_line() {
    let outcome = run("", r#"ubiq.log("stamp", ubiq.now()); 0"#);
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.lines[0].level, ConsoleLevel::Log);
    let stamp = outcome.lines[0]
        .text
        .strip_prefix("stamp ")
        .expect("the arguments are joined with a space");
    assert!(
        chrono::DateTime::parse_from_rfc3339(stamp).is_ok(),
        "not ISO-8601: {stamp}"
    );
}

/// Every starter the picker offers is one the page opens on, so every one of them has to parse in
/// the dialect it claims. The two that exist to be refused are excluded by name, and the runaway
/// is excluded because validating it is the point and running it costs five seconds.
#[test]
fn every_starter_validates_in_its_own_dialect() {
    for example in ubiq::state::sink::SCRIPT_EXAMPLES {
        let report = script::validate(
            example.prelude,
            example.source,
            example.dialect,
            &OxcOptions::default(),
        );
        let expected_clean = !example.name.contains("Validate");
        assert_eq!(
            report.ok(),
            expected_clean,
            "{}: {:?}",
            example.name,
            report.errors
        );
    }
}

/// The starters that are meant to run, run — in both dialects, against their own preludes.
#[test]
fn every_runnable_starter_runs() {
    for example in ubiq::state::sink::SCRIPT_EXAMPLES {
        if example.name.contains("Validate") || example.name.contains("limit") {
            continue;
        }
        let outcome = script::eval(Run {
            prelude: example.prelude.to_string(),
            source: example.source.to_string(),
            dialect: example.dialect,
            ..Default::default()
        });
        assert_eq!(outcome.error, None, "{}: {outcome:?}", example.name);
    }
}

/// The reference is the page's only description of the surface, so every namespace it advertises
/// has to be there. A document that named a binding the interpreter does not install is worse
/// than no document.
#[test]
fn every_namespace_the_reference_names_exists() {
    for namespace in ["project", "tasks", "ai", "search", "sink"] {
        assert!(
            script::API_DOC.contains(&format!("`ubiq.{namespace}`")),
            "the reference does not name ubiq.{namespace}"
        );
        let outcome = run("", &format!("typeof ubiq.{namespace}"));
        assert_eq!(
            outcome.value.as_deref(),
            Some("object"),
            "ubiq.{namespace} is not installed"
        );
    }

    // Every call the reference writes in a table row, checked to be callable.
    for call in [
        "ubiq.version",
        "ubiq.platform",
        "ubiq.now",
        "ubiq.log",
        "ubiq.echo",
        "ubiq.project.info",
        "ubiq.project.root",
        "ubiq.project.list",
        "ubiq.tasks.list",
        "ubiq.tasks.get",
        "ubiq.tasks.create",
        "ubiq.tasks.cancel",
        "ubiq.ai.available",
        "ubiq.ai.models",
        "ubiq.ai.ask",
        "ubiq.search.last",
        "ubiq.search.files",
        "ubiq.search.text",
        "ubiq.sink.ui",
        "ubiq.sink.event",
        "ubiq.sink.clear",
    ] {
        assert!(
            script::API_DOC.contains(&format!("`{call}(")),
            "the reference does not document {call}"
        );
        let outcome = run("", &format!("typeof {call}"));
        assert_eq!(
            outcome.value.as_deref(),
            Some("function"),
            "{call} is not a function"
        );
    }
}

// ── The host surface ────────────────────────────────────────────────

/// What the interface would have captured before a run, filled in enough for every namespace to
/// have something to answer with.
fn facts() -> HostFacts {
    HostFacts {
        project: serde_json::json!({ "id": "p1", "name": "ubiq", "root": "/tmp/ubiq", "open": true }),
        projects: serde_json::json!([{ "id": "p1", "name": "ubiq", "root": "/tmp/ubiq" }]),
        tasks: serde_json::json!([
            { "id": "t1", "title": "wire the sink", "status": "running" },
            { "id": "t2", "title": "write it down", "status": "todo" },
        ]),
        models: serde_json::json!([{ "id": "local/qwen", "name": "Qwen", "local": true }]),
        search: serde_json::json!({
            "query": "TODO",
            "results": [
                { "path": "crates/ubiq/src/lib.rs", "hits": [] },
                { "path": "README.md", "hits": [] },
            ],
        }),
    }
}

fn with_facts(source: &str) -> script::ScriptOutcome {
    script::eval(Run {
        source: source.to_string(),
        facts: facts(),
        ..Default::default()
    })
}

/// The read-only namespaces answer from the snapshot, and answer the same thing twice: a script
/// that mutates what it was handed cannot corrupt what the next call returns.
#[test]
fn the_snapshot_is_what_a_script_reads() {
    let outcome = with_facts(
        r#"console.log(ubiq.project.info().name, ubiq.project.root());
           console.log(ubiq.project.list().length);
           console.log(ubiq.tasks.list().map((t) => t.id).join(","));
           console.log(ubiq.tasks.get("t2").title);
           console.log(ubiq.tasks.get("nope"));
           console.log(ubiq.ai.available(), ubiq.ai.models()[0].id);
           console.log(ubiq.search.last().query);
           console.log(ubiq.search.files(".rs").map((r) => r.path).join(","));
           ubiq.project.info().name = "clobbered";
           ubiq.project.info().name"#,
    );
    assert_eq!(outcome.error, None, "{outcome:?}");
    let lines: Vec<_> = outcome
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        lines,
        vec![
            "ubiq /tmp/ubiq",
            "1",
            "t1,t2",
            "write it down",
            "null",
            "true local/qwen",
            "TODO",
            "crates/ubiq/src/lib.rs",
        ]
    );
    assert_eq!(outcome.value.as_deref(), Some("ubiq"));
}

/// A page with nothing open answers with nothing rather than throwing, and the three lists answer
/// with an *empty list* rather than `null` — which is what lets a script map over them without
/// first asking whether a project is open.
#[test]
fn an_empty_snapshot_reads_the_same_as_a_full_one() {
    let outcome = run(
        "",
        r#"[
             ubiq.project.info() === null,
             ubiq.project.root() === null,
             ubiq.ai.available() === false,
             ubiq.project.list().length === 0,
             ubiq.tasks.list().map((t) => t.id).length === 0,
             ubiq.ai.models().length === 0,
             ubiq.search.files(".rs").length === 0,
             ubiq.search.last() === null,
           ].every(Boolean)"#,
    );
    assert_eq!(outcome.error, None, "{outcome:?}");
    assert_eq!(outcome.value.as_deref(), Some("true"));
}

/// Every call that would change something records what it was asked for and performs nothing.
#[test]
fn a_write_is_an_intent_and_nothing_more() {
    let outcome = with_facts(
        r#"ubiq.tasks.create({ title: "tidy" });
           ubiq.tasks.cancel("t1");
           ubiq.ai.ask("summarise", { model: "local/qwen" });
           ubiq.search.text("TODO");
           "done""#,
    );
    assert_eq!(outcome.error, None, "{outcome:?}");
    assert_eq!(outcome.value.as_deref(), Some("done"));

    let calls: Vec<_> = outcome
        .intents
        .iter()
        .map(|intent| (intent.namespace, intent.call))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("tasks", "create"),
            ("tasks", "cancel"),
            ("ai", "ask"),
            ("search", "text"),
        ]
    );
    assert_eq!(outcome.intents[0].args, r#"{"title":"tidy"}"#);
    assert_eq!(
        outcome.intents[2].args,
        r#""summarise", {"model":"local/qwen"}"#
    );
    assert!(outcome.intents[0].line().contains("not performed"));
}

/// An intent answers `null`, so a script that treats it as a result gets one it can branch on
/// rather than an exception at the call site.
#[test]
fn an_intent_answers_null() {
    let outcome = run("", r#"ubiq.ai.ask("anything") === null"#);
    assert_eq!(outcome.value.as_deref(), Some("true"));
}

// ── The declared panel ──────────────────────────────────────────────

const PANEL: &str = r#"{ createSurface: { surfaceId: "s", components: [], dataModel: {} } }"#;

#[test]
fn a_declaration_comes_back_as_a_payload() {
    let outcome = run("", &format!("ubiq.sink.ui({PANEL}); 0"));
    assert_eq!(outcome.error, None, "{outcome:?}");
    let ui = outcome.ui.expect("the declaration is reported");
    assert!(!ui.handler, "no handler was passed");
    assert!(ui.payload.contains(r#""surfaceId":"s""#), "{}", ui.payload);
}

#[test]
fn a_declaration_may_be_a_json_string() {
    let outcome = run(
        "",
        &format!("ubiq.sink.ui(JSON.stringify({PANEL}), () => {{}}); 0"),
    );
    let ui = outcome.ui.expect("the declaration is reported");
    assert!(ui.handler, "a handler was passed");
    assert!(ui.payload.contains(r#""surfaceId":"s""#), "{}", ui.payload);
}

/// Declaring twice replaces, and `clear` withdraws: the page draws the last thing the run said,
/// not the first.
#[test]
fn the_last_declaration_wins_and_clear_withdraws() {
    let outcome = run(
        "",
        &format!(
            r#"ubiq.sink.ui({PANEL}); ubiq.sink.ui({{ createSurface: {{ surfaceId: "t" }} }}); 0"#
        ),
    );
    let ui = outcome.ui.expect("a declaration");
    assert!(ui.payload.contains(r#""surfaceId":"t""#), "{}", ui.payload);

    let outcome = run("", &format!("ubiq.sink.ui({PANEL}); ubiq.sink.clear(); 0"));
    assert!(outcome.ui.is_none(), "clear withdraws the panel");
}

/// A plain run sees no event; a replay sees the one it was given, and the handler is called with
/// it after the script has finished declaring.
#[test]
fn a_replay_reaches_the_handler() {
    let source = format!(
        r#"const e = ubiq.sink.event();
           console.log("event:", e ? e.name : "none");
           ubiq.sink.ui({PANEL}, (got) => console.log("handler:", got.name, got.componentId));
           0"#
    );

    let plain = run("", &source);
    assert_eq!(plain.error, None, "{plain:?}");
    assert_eq!(plain.lines[0].text, "event: none");
    assert_eq!(plain.lines.len(), 1, "no handler runs without an event");

    let replayed = script::eval(Run {
        source,
        event: Some(ScriptEvent {
            name: "pressed".to_string(),
            surface_id: "s".to_string(),
            component_id: "go".to_string(),
        }),
        ..Default::default()
    });
    assert_eq!(replayed.error, None, "{replayed:?}");
    assert_eq!(replayed.lines[0].text, "event: pressed");
    assert_eq!(replayed.lines[1].text, "handler: pressed go");
}

/// A handler that throws is the run's error, not a silence: the press has to say it failed.
#[test]
fn a_handler_that_throws_is_the_runs_error() {
    let outcome = script::eval(Run {
        source: format!(
            r#"ubiq.sink.ui({PANEL}, () => {{ throw new Error("handler broke"); }}); 0"#
        ),
        event: Some(ScriptEvent {
            name: "pressed".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    let error = outcome.error.expect("the handler's throw is the error");
    assert!(error.contains("handler broke"), "{error}");
}

// ── TypeScript compilation ──────────────────────────────────────────

/// Types compile away to plain JavaScript: the annotations, the type-space declarations, the `as`
/// and the `satisfies` are all gone, and a program that ran before runs afterwards.
#[test]
fn plain_type_scripts_compile_to_runnable_javascript() {
    let typed = r#"
interface Box { label: string; size: number }
type Id = string;

function make(label: string, size: number): Box {
  const box: Box = { label, size } as Box;
  return box satisfies Box;
}

let id: Id = "box-1" as Id;
console.log(make(id, 2));
"#;
    let compiled =
        script::transpile(typed, Dialect::Ts, &OxcOptions::default()).expect("TypeScript compiles");
    assert!(!compiled.contains("interface"), "{compiled}");
    assert!(!compiled.contains("as Box"), "{compiled}");
    assert!(!compiled.contains("satisfies"), "{compiled}");
    assert!(!compiled.contains(": string"), "{compiled}");

    let outcome = run("", &compiled);
    assert_eq!(outcome.error, None);
    assert_eq!(
        outcome.lines[0].text, r#"{"label":"box-1","size":2}"#,
        "{outcome:?}"
    );
}

/// The prelude and the source are both compiled before the run: that is what makes the whole page
/// TypeScript, not just the script half.
#[test]
fn a_typed_prelude_compiles_with_the_source() {
    let prelude = r#"
interface Greeting { who: string }
function greet(g: Greeting): string {
  return `hello, ${g.who}`;
}
"#;
    let outcome = run_ts(prelude, r#"greet({ who: "Ubiq" })"#);
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.value.as_deref(), Some("hello, Ubiq"));
}

/// Optional parameters are the one annotation whose `?` is punctuation, and a compiler that left
/// it would produce `x?` — not JavaScript.
#[test]
fn an_optional_parameter_drops_its_question_mark() {
    let compiled = script::transpile(
        "function f(x?: number) {}",
        Dialect::Ts,
        &OxcOptions::default(),
    )
    .expect("compiles");
    assert_eq!(compiled.trim(), "function f(x) {}");
}

/// The constructs with a runtime body — an enum, a namespace, a generic call — are lowered rather
/// than refused: oxc emits the JavaScript they mean, and it runs.
#[test]
fn a_construct_with_a_runtime_body_is_lowered() {
    let outcome = run_ts("", "enum Colour { Red, Green }\nColour[Colour.Green];");
    assert_eq!(outcome.error, None, "{outcome:?}");
    assert_eq!(outcome.value.as_deref(), Some("Green"));

    let outcome = run_ts(
        "namespace Shapes { export const sides = 3; }",
        "Shapes.sides",
    );
    assert_eq!(outcome.error, None, "{outcome:?}");
    assert_eq!(outcome.value.as_deref(), Some("3"));
}

// ── The oxc settings ────────────────────────────────────────────────

/// The settings the page's panel writes reach the front end: a target lowers, a module parses an
/// import, and JSX is a syntax error until it is asked for.
#[test]
fn the_settings_reach_the_front_end() {
    // Nullish coalescing is ES2020, so an ES2017 target has to lower it and an ESNext target has
    // to leave it exactly as written.
    let nullish = "const f = (a, b) => a ?? b;";

    let old = OxcOptions {
        target: EsTarget::Es2017,
        transform_js: true,
        ..Default::default()
    };
    let lowered = script::transpile(nullish, Dialect::Js, &old).expect("compiles");
    assert!(
        !lowered.contains("??"),
        "es2017 did not lower the nullish coalescing: {lowered}"
    );
    assert!(
        script::transpile(nullish, Dialect::Js, &OxcOptions::default())
            .expect("compiles")
            .contains("??"),
        "esnext lowered something it should not have"
    );

    let jsx = "const v = <div>hi</div>;";
    assert!(!script::validate("", jsx, Dialect::Js, &OxcOptions::default()).ok());
    let with_jsx = OxcOptions {
        jsx: true,
        ..Default::default()
    };
    assert!(
        script::validate("", jsx, Dialect::Js, &with_jsx).ok(),
        "jsx did not parse with the switch on"
    );
}

/// `transform_js` is what makes a JavaScript run go through the transformer at all, and the page
/// promises that what Validate accepts is what Run compiles.
///
/// Exponentiation rather than a private field on purpose: it lowers to `Math.pow`, which the
/// context defines, where a private field lowers to helper functions oxc expects a bundler to
/// supply. `EsTarget` says as much — lowering far below the interpreter's own level is a thing
/// the page lets a reader do and then reports honestly, not a thing it promises will run.
#[test]
fn lowering_javascript_still_runs_it() {
    let outcome = script::eval(Run {
        source: "const square = (n) => n ** 2; square(6) + 6".to_string(),
        options: OxcOptions {
            target: EsTarget::Es2015,
            transform_js: true,
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(outcome.error, None, "{outcome:?}");
    assert_eq!(outcome.value.as_deref(), Some("42"));
}

// ── Syntax validation ───────────────────────────────────────────────

#[test]
fn clean_javascript_validates() {
    let report = script::validate(
        "",
        "const rows = [1, 2, 3].map((n) => n * 2);",
        Dialect::Js,
        &OxcOptions::default(),
    );
    assert!(report.ok(), "{:?}", report.errors);
    assert_eq!(report.dialect, Dialect::Js);
    assert!(report.verdict().starts_with("ok"), "{}", report.verdict());
}

#[test]
fn broken_javascript_is_named_first() {
    let report = script::validate("", "function (", Dialect::Js, &OxcOptions::default());
    assert!(!report.ok());
    let first = &report.errors[0];
    assert_eq!(first.buffer, script::Buffer::Script);
    assert_eq!(first.line, 1, "{:?}", report.errors);
    assert!(!first.message.is_empty(), "{:?}", report.errors);
    assert!(report.verdict().contains("problem"), "{}", report.verdict());
}

/// The page has two buffers, and a report that did not say which one a line number belongs to
/// would send the reader to the wrong place. `check_prelude` is what decides whether it looks.
#[test]
fn a_broken_prelude_is_named_as_the_prelude() {
    let report = script::validate("function (", "1 + 1", Dialect::Js, &OxcOptions::default());
    assert!(!report.ok());
    assert_eq!(report.errors[0].buffer, script::Buffer::Prelude);

    let unchecked = OxcOptions {
        check_prelude: false,
        ..Default::default()
    };
    assert!(script::validate("function (", "1 + 1", Dialect::Js, &unchecked).ok());
}

#[test]
fn clean_typescript_validates_after_compiling() {
    let report = script::validate(
        "",
        "interface P { x: number }\nconst p: P = { x: 1 };",
        Dialect::Ts,
        &OxcOptions::default(),
    );
    assert!(report.ok(), "{:?}", report.errors);
}

/// An enum is TypeScript this page can run, so it validates. A type that is not a type is not,
/// and the report says which line it is on.
#[test]
fn broken_typescript_is_placed_on_its_line() {
    assert!(script::validate("", "enum Wat { A }", Dialect::Ts, &OxcOptions::default()).ok());

    let report = script::validate(
        "",
        "const x: number = 1;\nconst y: = 2;",
        Dialect::Ts,
        &OxcOptions::default(),
    );
    assert!(!report.ok());
    assert_eq!(report.errors[0].line, 2, "{:?}", report.errors);
}

/// JavaScript is parsed as JavaScript: an annotation is a syntax error, not a type erased in
/// silence.
#[test]
fn typescript_does_not_validate_as_javascript() {
    let report = script::validate(
        "",
        "const x: number = 1;",
        Dialect::Js,
        &OxcOptions::default(),
    );
    assert!(!report.ok(), "{report:?}");
}

/// A truly exploded file does not need every red square, and the ceiling is the page's to choose.
#[test]
fn a_report_is_cut_at_the_ceiling() {
    let broken = "const x: = 1;\n".repeat(20);
    let capped = OxcOptions {
        max_errors: 3,
        ..Default::default()
    };
    let report = script::validate("", &broken, Dialect::Ts, &capped);
    assert!(report.errors.len() <= 3, "{:?}", report.errors);
}
