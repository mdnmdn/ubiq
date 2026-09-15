//! A script, run once: source in, console lines, host intents and a completion value out.
//!
//! **This is not a plugin system.** It is the interpreter behind the sink's script page, and it
//! holds nothing between runs — every call builds a fresh runtime and a fresh context, and drops
//! both before it answers. Nothing here is registered, nothing here is persisted, and no script
//! can reach a pane, a project, a window or the bus.
//!
//! What a script *can* reach is a [`HostFacts`] snapshot the caller captured before the run, and
//! an [`Intent`] log it writes into. That split is the whole of the capability model:
//!
//! - **A read is a fact.** `ubiq.project`, `ubiq.tasks`, `ubiq.ai` and `ubiq.search` answer from
//!   JSON the interface serialised on its own thread before the interpreter existed. A script
//!   cannot block on the interface, and the interface cannot be re-entered from a script.
//! - **A write is an intent.** `ubiq.tasks.create`, `ubiq.ai.ask` and `ubiq.search.text` record
//!   what was asked for and perform nothing — the same rule `D115` puts on an A2UI local call.
//! - **A surface is a declaration.** `ubiq.sink.ui(payload, handler)` names the panel the page
//!   should draw. Because nothing survives a run, a click on that panel **re-runs the script**
//!   with the event in hand and the handler is called then. The script is the state.
//!
//! `_docs/inbox/plugin-system-proposal.md` is where a plugin system would build on this — a stored
//! script, a trigger, a surface to bind to. None of that exists, and none of it is implied by what
//! is here.
//!
//! The interpreter is QuickJS ([`rquickjs`]), compiled into the process behind the `quickjs`
//! feature. Off it, every entry point below still exists: [`available`] is false, [`engine_name`]
//! is `"none"`, and [`eval`] answers with an outcome whose error says so. The page reads the same
//! either way, which is what lets the feature be off without a second version of the screen.
//! [`transpile`] and [`validate`] go through `oxc` — a real parser and transformer, needing no
//! interpreter, so both exist in every build, and [`OxcOptions`] is what the page's settings panel
//! turns to steer them.
//!
//! **A script never runs on the thread that called it.** Each run gets its own thread, and the
//! caller waits for an answer on it. Two limits keep that honest — a per-step wall-clock interrupt
//! and a ceiling on the whole run — and neither is a sandbox claim beyond what it says: a script
//! that runs too long is stopped, and a script that allocates too much throws.

use serde_json::Value;

/// Which dialect a run or a validation is given. JavaScript is parsed as
/// written; TypeScript is compiled to JavaScript first.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Dialect {
    #[default]
    Js,
    Ts,
}

impl Dialect {
    /// What the toggle row and the chrome call it.
    pub fn label(self) -> &'static str {
        match self {
            Dialect::Js => "JavaScript",
            Dialect::Ts => "TypeScript",
        }
    }

    /// The short form, for a control with no room for the long one.
    pub fn short(self) -> &'static str {
        match self {
            Dialect::Js => "JS",
            Dialect::Ts => "TS",
        }
    }

    /// The two, in toggle order.
    pub fn all() -> &'static [Dialect] {
        &[Dialect::Js, Dialect::Ts]
    }
}

/// Which ECMAScript level the transformer lowers to. `EsNext` is "lower nothing but the types".
///
/// Five of oxc's targets rather than all of them, because the panel is a row of pills and the
/// levels between these lower nothing a reader of this page would notice. ES5 is not among them:
/// oxc refuses it outright, and a pill that silently did nothing would be worse than its absence.
///
/// **Lowering far below the interpreter's own level emits helper calls.** QuickJS is roughly
/// ES2023, and a program lowered to ES2015 can come out calling `_classPrivateFieldInitSpec` and
/// friends — functions oxc expects a bundler to supply and that nothing in this context defines.
/// That is a real property of the transformer, not a bug here: the page compiles what it is asked
/// to compile, and a run that then throws on a missing helper says so in the console.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EsTarget {
    Es2015,
    Es2017,
    Es2020,
    Es2022,
    #[default]
    EsNext,
}

impl EsTarget {
    /// What the settings panel calls it, and what oxc calls it.
    pub fn label(self) -> &'static str {
        match self {
            EsTarget::Es2015 => "es2015",
            EsTarget::Es2017 => "es2017",
            EsTarget::Es2020 => "es2020",
            EsTarget::Es2022 => "es2022",
            EsTarget::EsNext => "esnext",
        }
    }

    /// The five, in the order the panel lists them.
    pub fn all() -> &'static [EsTarget] {
        &[
            EsTarget::Es2015,
            EsTarget::Es2017,
            EsTarget::Es2020,
            EsTarget::Es2022,
            EsTarget::EsNext,
        ]
    }
}

/// How oxc is pointed at a program: what the source is allowed to contain, what the transformer
/// lowers, and how much of a broken file a report names.
///
/// These are the front end's dials and nothing else — none of them reaches QuickJS, which runs
/// whatever JavaScript comes out the other side. The page's settings panel writes this and the
/// Run and Validate buttons both read it, so what Validate accepts is what Run compiles.
///
/// There is deliberately no "parse as a module" switch: oxc's parser accepts `import` and `export`
/// in either mode, so the pill would have been a dial with nothing behind it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OxcOptions {
    /// Allow JSX syntax. The transformer lowers it to `React.createElement` calls, which nothing
    /// in the context defines — it is here to exercise the front end, not to render anything.
    pub jsx: bool,
    /// What the transformer lowers to. Only consulted when something is being transformed.
    pub target: EsTarget,
    /// Run the transformer over JavaScript as well, so `target` lowers a JavaScript program too.
    /// Off by default: JavaScript is handed to QuickJS as written unless this is asked for.
    pub transform_js: bool,
    /// Validate the prelude as well as the program. On by default, because a prelude that does
    /// not compile is as much the run's error as a program that does not.
    pub check_prelude: bool,
    /// How many problems one report names. A truly exploded file does not need every red square.
    pub max_errors: usize,
}

impl Default for OxcOptions {
    fn default() -> Self {
        Self {
            jsx: false,
            target: EsTarget::default(),
            transform_js: false,
            check_prelude: true,
            max_errors: 30,
        }
    }
}

impl OxcOptions {
    /// The panel's summary line: what is currently on, in the order the panel lists it.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("target {}", self.target.label())];
        if self.jsx {
            parts.push("jsx".to_string());
        }
        if self.transform_js {
            parts.push("transform js".to_string());
        }
        if self.check_prelude {
            parts.push("check prelude".to_string());
        }
        parts.push(format!("max {} errors", self.max_errors));
        parts.join(" \u{00b7} ")
    }
}

/// How long one uninterrupted stretch of script execution may take before the interrupt handler
/// stops it. This is the "blocking" ceiling: it protects the page from `while (true) {}`.
pub const BLOCK_LIMIT_MS: u64 = 5_000;

/// How long a whole run may take, from the moment it is asked for to the answer. The interpreter
/// runs on its own thread, so this is the ceiling on how long that thread is waited on: a runtime
/// too sick even for its own interrupt to fire is abandoned rather than waited out forever.
pub const TOTAL_LIMIT_MS: u64 = 15_000;

/// How much a script may allocate before QuickJS throws out of memory. The interpreter shares the
/// interface's address space, so a runaway allocation is the interface's problem: 16 MiB is far
/// past anything the page's own starter needs and far short of anything that hurts.
pub const MEMORY_LIMIT: usize = 16 * 1024 * 1024;

/// Which of the four console instructions a line came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConsoleLevel {
    Log,
    Info,
    Warn,
    Error,
}

/// One console call, with its arguments already formatted and joined.
#[derive(Clone, Debug)]
pub struct ConsoleLine {
    pub level: ConsoleLevel,
    pub text: String,
}

/// Everything the interface knows that a script may read, serialised before the run.
///
/// Every field is a JSON document the interface built on its own thread. A script gets a parse of
/// it and nothing more: there is no handle behind any of these, so a script cannot reach the
/// interface, block it, or see it change mid-run.
#[derive(Clone, Debug)]
pub struct HostFacts {
    /// `{ id, name, root, open }` for the project in front, or `null` when none is.
    pub project: Value,
    /// Every project the catalogue knows, as `{ id, name, root }`.
    pub projects: Value,
    /// The tasks in flight, newest first.
    pub tasks: Value,
    /// The inference models this build can reach, and whether any can be.
    pub models: Value,
    /// The last search the interface ran: `{ query, kind, results }`.
    pub search: Value,
}

/// Nothing known, in the shape each namespace answers in.
///
/// The three lists default to *empty lists* rather than to `null`, which is the whole point of
/// writing this out: a script that maps over `ubiq.tasks.list()` has to read the same on a page
/// with no project open as on one with fifty tasks. Only the two facts that are genuinely
/// singular — the front project, the last search — are `null` when there is none, because there
/// `?.` is the right thing for a script to write.
impl Default for HostFacts {
    fn default() -> Self {
        Self {
            project: Value::Null,
            projects: Value::Array(Vec::new()),
            tasks: Value::Array(Vec::new()),
            models: Value::Array(Vec::new()),
            search: Value::Null,
        }
    }
}

impl HostFacts {
    /// One document per namespace, as the `ubiq` global will hand it out. A missing fact is
    /// `null` rather than absent, so a script's `?.` reads the same whether or not a project is
    /// open.
    #[cfg_attr(not(feature = "quickjs"), allow(dead_code))]
    fn or_null(value: &Value) -> String {
        if value.is_null() {
            "null".to_string()
        } else {
            value.to_string()
        }
    }
}

/// One thing a script asked the interface to do, and that the interface did not do.
///
/// The interpreter performs no effect. A call that would change something anywhere outside its own
/// context lands here instead, is drawn in the console, and stops — the same rule an A2UI local
/// call is held to (`D115`). What makes it useful is that the arguments are kept verbatim, so the
/// intent is a readable account of what a plugin system would have to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    /// The namespace it came from: `tasks`, `ai`, `search`, `sink`.
    pub namespace: &'static str,
    /// The call, without its namespace: `create`, `ask`, `text`.
    pub call: &'static str,
    /// The arguments, as JSON.
    pub args: String,
}

impl Intent {
    /// The one line the console prints for it.
    pub fn line(&self) -> String {
        format!(
            "intent: ubiq.{}.{}({}) \u{2014} recorded, not performed",
            self.namespace, self.call, self.args
        )
    }
}

/// The panel a script declared through `ubiq.sink.ui`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptUi {
    /// The A2UI payload, as JSON. Handed to `state::a2ui::parse` exactly as the A2UI page's own
    /// editor contents are.
    pub payload: String,
    /// Whether the declaration came with a handler. Without one, a click on the panel has nowhere
    /// to go and the page says so rather than re-running for nothing.
    pub handler: bool,
}

/// An event on a declared panel, on its way back into a fresh run.
///
/// Replay, not resumption: the run that drew the panel is long gone, so the script is evaluated
/// again from the top with this in hand, and the handler it registers is called at the end. A
/// script that wants a click to mean something therefore has to be idempotent, which is the same
/// discipline the page's "nothing survives between runs" already imposes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptEvent {
    /// The event name the button's action carried.
    pub name: String,
    /// The surface it fired on.
    pub surface_id: String,
    /// The component that fired it.
    pub component_id: String,
}

impl ScriptEvent {
    /// The object the handler is called with, and that `ubiq.sink.event()` answers.
    #[cfg_attr(not(feature = "quickjs"), allow(dead_code))]
    fn as_json(&self) -> String {
        serde_json::json!({
            "name": self.name,
            "surfaceId": self.surface_id,
            "componentId": self.component_id,
        })
        .to_string()
    }
}

/// Everything one run of [`eval`] produced.
#[derive(Clone, Debug, Default)]
pub struct ScriptOutcome {
    /// The console lines, in the order the script wrote them.
    pub lines: Vec<ConsoleLine>,
    /// The completion value, formatted. `None` when the script threw or returned undefined.
    pub value: Option<String>,
    /// The exception, with its message and its stack when there is one.
    pub error: Option<String>,
    /// What the script asked for and did not get, in the order it asked.
    pub intents: Vec<Intent>,
    /// The panel the script declared, if it declared one.
    pub ui: Option<ScriptUi>,
    /// How long the whole run took, from being asked for to answering.
    pub elapsed_ms: u64,
}

/// Which buffer a problem is in. The page has two, and a report that did not say which would send
/// the reader to the wrong line number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Buffer {
    Prelude,
    Script,
}

impl Buffer {
    pub fn label(self) -> &'static str {
        match self {
            Buffer::Prelude => "prelude",
            Buffer::Script => "script",
        }
    }
}

/// One problem [`validate`] found. Mutable so a caller can cheaply add to a report.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// Which of the page's two buffers it is in.
    pub buffer: Buffer,
    /// The one-based line the problem starts on.
    pub line: u32,
    /// The one-based column it starts at.
    pub column: u32,
    /// What is wrong, in a line a monitor can read.
    pub message: String,
    /// The offending source, trimmed, when the parser knows what that is.
    pub snippet: String,
}

/// What [`validate`] decided about a program it did not run.
#[derive(Clone, Debug, Default)]
pub struct SyntaxReport {
    pub dialect: Dialect,
    /// The problems, in reading order. Empty is `ok()`.
    pub errors: Vec<Diagnostic>,
    /// How many lines were checked, across both buffers.
    pub lines_checked: usize,
    /// How long the parse and the transform took.
    pub elapsed_ms: u64,
}

impl SyntaxReport {
    /// Whether the program parsed and compiled clean, with nothing oxc would refuse.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// The one line the console prints when it is clean.
    pub fn verdict(&self) -> String {
        if self.ok() {
            format!(
                "ok \u{2014} {} parsed and compiled clean, {} line{} checked in {} ms",
                self.dialect.label(),
                self.lines_checked,
                if self.lines_checked == 1 { "" } else { "s" },
                self.elapsed_ms
            )
        } else {
            format!(
                "{} problem{} in {} \u{2014} checked in {} ms",
                self.errors.len(),
                if self.errors.len() == 1 { "" } else { "s" },
                self.dialect.label(),
                self.elapsed_ms
            )
        }
    }
}

/// The reference the page draws under its console, as Markdown.
///
/// One document rather than a table of pairs: the surface has namespaces now, and a namespace
/// needs a paragraph and an example to be worth having. Markdown is what makes that cheap to keep
/// current — the page renders this string through the same viewer a `.md` file goes through, so
/// adding a binding is adding a bullet here and nothing else.
pub const API_DOC: &str = r#"# `ubiq` — what a script may call

Every run gets a **fresh context**. Nothing survives between runs: no variable, no timer, no
listener. What a script reads is a snapshot the interface serialised before the run started; what
a script writes is an **intent**, recorded and drawn in the console, and performed by nobody.

## Core

| Call | Answers |
| --- | --- |
| `ubiq.version()` | The interface's version string, the one the footer prints. |
| `ubiq.platform()` | The operating system this build runs on: `macos`, `linux`, `windows`. |
| `ubiq.now()` | The wall clock as an ISO-8601 timestamp, in UTC. |
| `ubiq.log(...args)` | The same as `console.log`: one line in the console below. |
| `ubiq.echo(value)` | The value's formatted form, returned as a string. |

`console.log`, `console.info`, `console.warn` and `console.error` all write to the console, each at
its own colour.

## `ubiq.project` — where the interface is pointed

| Call | Answers |
| --- | --- |
| `ubiq.project.info()` | `{ id, name, root, open }` for the project in front, or `null`. |
| `ubiq.project.root()` | The front project's folder, or `null`. |
| `ubiq.project.list()` | Every project the catalogue knows: `{ id, name, root }`. |

```js
const p = ubiq.project.info();
console.log(p ? `${p.name} at ${p.root}` : "no project open");
```

## `ubiq.tasks` — what is in flight

| Call | Does |
| --- | --- |
| `ubiq.tasks.list()` | The tasks in flight, newest first. |
| `ubiq.tasks.get(id)` | One of them by id, or `null`. |
| `ubiq.tasks.create(spec)` | **Intent.** Records the task it would have started. |
| `ubiq.tasks.cancel(id)` | **Intent.** Records the cancellation it would have sent. |

```js
const running = ubiq.tasks.list().filter((t) => t.state === "running");
console.log(`${running.length} running`);
ubiq.tasks.create({ title: "tidy the docs", project: ubiq.project.info()?.id });
```

## `ubiq.ai` — local models and inference

| Call | Does |
| --- | --- |
| `ubiq.ai.available()` | Whether this build can reach any model at all. |
| `ubiq.ai.models()` | The models it can reach: `{ id, name, provider, local }`. |
| `ubiq.ai.ask(prompt, opts)` | **Intent.** Records the inference it would have run; answers `null`. |

```js
if (ubiq.ai.available()) {
  console.log(ubiq.ai.models().map((m) => m.id));
  ubiq.ai.ask("summarise the diff", { model: "local/qwen", maxTokens: 256 });
}
```

## `ubiq.search` — the last query, and the next one

| Call | Does |
| --- | --- |
| `ubiq.search.last()` | The interface's last search: `{ query, kind, results }`. |
| `ubiq.search.files(pattern)` | The file hits of that search whose path matches `pattern`. |
| `ubiq.search.text(query, opts)` | **Intent.** Records the content search it would have run. |

```js
console.log(ubiq.search.files(".rs").slice(0, 5));
ubiq.search.text("TODO", { caseSensitive: false });
```

## `ubiq.sink` — a panel of the script's own

`ubiq.sink.ui(payload, handler)` declares an **A2UI surface** the page draws in the panel beside
this document — flip the switch above from *Docs* to *Panel* to see it. The payload is exactly what
the A2UI page's editor takes.

| Call | Does |
| --- | --- |
| `ubiq.sink.ui(payload, handler?)` | Declares the panel. `payload` may be an object or a JSON string. |
| `ubiq.sink.event()` | The event being replayed, or `null` on a plain Run. |
| `ubiq.sink.clear()` | Withdraws the panel. |

**A click re-runs the script.** Nothing survives a run, so there is no closure left to call: the
page evaluates the script again from the top with the event in hand, and calls the handler the
second declaration registers. Branch on `ubiq.sink.event()` and keep the script idempotent.

```js
const event = ubiq.sink.event();

ubiq.sink.ui({
  createSurface: {
    surfaceId: "hello",
    components: [
      { id: "root", component: "Card", child: "col" },
      { id: "col", component: "Column", children: ["msg", "go"] },
      { id: "msg", component: "Text", text: { path: "/msg" } },
      { id: "label", component: "Text", text: "press me" },
      { id: "go", component: "Button", child: "label",
        variant: "primary", action: { event: { name: "pressed" } } }
    ],
    dataModel: { msg: event ? `pressed at ${ubiq.now()}` : "nothing pressed yet" }
  }
}, (e) => console.log("handler saw", e.name, "on", e.componentId));
```
"#;

#[cfg(feature = "quickjs")]
mod engine {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

    use rquickjs::function::{Opt, Rest};
    use rquickjs::{Context, Ctx, Function, Object, Runtime, Value};

    use super::{
        ConsoleLevel, ConsoleLine, Dialect, HostFacts, Intent, OxcOptions, ScriptEvent,
        ScriptOutcome, ScriptUi, front,
    };

    /// What the installed globals write into, shared with the run that will report it.
    #[derive(Default)]
    struct Recorder {
        lines: Vec<ConsoleLine>,
        intents: Vec<Intent>,
        ui: Option<ScriptUi>,
    }

    type Shared = Rc<RefCell<Recorder>>;

    /// One run, on the thread that owns the runtime. Everything here stays on that thread; the
    /// only thing that crosses it is the [`ScriptOutcome`] it returns.
    pub fn eval(
        prelude: &str,
        source: &str,
        dialect: Dialect,
        options: OxcOptions,
        facts: &HostFacts,
        event: Option<&ScriptEvent>,
        block_ms: u64,
    ) -> ScriptOutcome {
        let started = Instant::now();
        let shared: Shared = Rc::new(RefCell::new(Recorder::default()));

        let (value, error) = match run(
            prelude, source, dialect, options, facts, event, &shared, started, block_ms,
        ) {
            Ok(value) => (value, None),
            Err(message) => (None, Some(message)),
        };

        let recorder = shared.borrow();
        ScriptOutcome {
            lines: recorder.lines.clone(),
            value,
            error,
            intents: recorder.intents.clone(),
            ui: recorder.ui.clone(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }

    /// The run itself. Every failure is a `String` — nothing here unwraps anything the script can
    /// reach, and nothing here panics.
    #[allow(clippy::too_many_arguments)]
    fn run(
        prelude: &str,
        source: &str,
        dialect: Dialect,
        options: OxcOptions,
        facts: &HostFacts,
        event: Option<&ScriptEvent>,
        shared: &Shared,
        started: Instant,
        block_ms: u64,
    ) -> Result<Option<String>, String> {
        // TypeScript is a surface language: QuickJS parses JavaScript, so the program is
        // compiled down to it before the interpreter ever sees either string. A prelude that does
        // not compile is as much the error as a script that does not. JavaScript takes the same
        // path only when the settings ask for it, so the default costs nothing.
        let transform = dialect == Dialect::Ts || options.transform_js;
        let (prelude, source) = if transform {
            (
                front::transpile(prelude, dialect, &options)
                    .map_err(|e| format!("prelude: {e}"))?,
                front::transpile(source, dialect, &options).map_err(|e| format!("script: {e}"))?,
            )
        } else {
            (prelude.to_string(), source.to_string())
        };

        let runtime = Runtime::new().map_err(|e| format!("could not start QuickJS: {e}"))?;
        runtime.set_memory_limit(super::MEMORY_LIMIT);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            started.elapsed().as_millis() as u64 >= block_ms
        })));

        let context =
            Context::full(&runtime).map_err(|e| format!("could not start QuickJS: {e}"))?;

        context.with(|ctx| {
            // The registered handler is the one JavaScript value this module holds in Rust, and
            // QuickJS asserts on a runtime freed while a value of its own is still referenced. So
            // it is emptied before the context is left, on every path — which is what the inner
            // closure and the unconditional clear below are for.
            let handler = Rc::new(RefCell::new(None::<Function>));
            let outcome = (|| {
                install(&ctx, shared, facts, event, &handler)
                    .map_err(|e| format!("could not install the globals: {e}"))?;

                if !prelude.trim().is_empty() {
                    // A prelude that throws is the error, and the source is not run.
                    ctx.eval::<Value, _>(prelude.as_str())
                        .map_err(|e| describe_error(&ctx, &e, started, block_ms))?;
                }

                let value = ctx
                    .eval::<Value, _>(source.as_str())
                    .map_err(|e| describe_error(&ctx, &e, started, block_ms))?;

                // The handler runs last, after the script has declared whatever it declares — a
                // declaration made after the call would otherwise never be seen by it.
                if let Some(event) = event {
                    let registered = handler.borrow().clone();
                    if let Some(function) = registered {
                        let payload = ctx
                            .json_parse(event.as_json())
                            .map_err(|e| describe_error(&ctx, &e, started, block_ms))?;
                        function
                            .call::<_, Value>((payload,))
                            .map_err(|e| describe_error(&ctx, &e, started, block_ms))?;
                    }
                }

                Ok(if value.is_undefined() {
                    None
                } else {
                    Some(format_value(&ctx, &value))
                })
            })();

            *handler.borrow_mut() = None;
            outcome
        })
    }

    /// `console` and `ubiq`, the whole of what a script can reach beyond the language itself.
    fn install<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        facts: &HostFacts,
        event: Option<&ScriptEvent>,
        handler: &Rc<RefCell<Option<Function<'js>>>>,
    ) -> rquickjs::Result<()> {
        let globals = ctx.globals();

        let console = Object::new(ctx.clone())?;
        for (name, level) in [
            ("log", ConsoleLevel::Log),
            ("info", ConsoleLevel::Info),
            ("warn", ConsoleLevel::Warn),
            ("error", ConsoleLevel::Error),
        ] {
            console.set(name, writer(ctx, shared, level)?)?;
        }
        globals.set("console", console)?;

        let ubiq = Object::new(ctx.clone())?;
        ubiq.set(
            "version",
            Function::new(ctx.clone(), || crate::version::FULL)?,
        )?;
        ubiq.set(
            "platform",
            Function::new(ctx.clone(), || std::env::consts::OS)?,
        )?;
        ubiq.set(
            "now",
            Function::new(ctx.clone(), || {
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            })?,
        )?;
        ubiq.set("log", writer(ctx, shared, ConsoleLevel::Log)?)?;
        ubiq.set(
            "echo",
            Function::new(ctx.clone(), |ctx: Ctx<'js>, value: Value<'js>| {
                format_value(&ctx, &value)
            })?,
        )?;

        ubiq.set("project", project(ctx, facts)?)?;
        ubiq.set("tasks", tasks(ctx, shared, facts)?)?;
        ubiq.set("ai", ai(ctx, shared, facts)?)?;
        ubiq.set("search", search(ctx, shared, facts)?)?;
        ubiq.set("sink", sink(ctx, shared, event, handler)?)?;

        globals.set("ubiq", ubiq)?;

        Ok(())
    }

    /// A function that answers one captured fact, freshly parsed each call so a script that
    /// mutates what it got cannot corrupt what the next call answers.
    fn fact<'js>(ctx: &Ctx<'js>, json: String) -> rquickjs::Result<Function<'js>> {
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            ctx.json_parse(json.clone())
                .unwrap_or_else(|_| Value::new_null(ctx.clone()))
        })
    }

    /// A function that records an intent and performs nothing, answering `null`.
    fn intent<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        namespace: &'static str,
        call: &'static str,
    ) -> rquickjs::Result<Function<'js>> {
        let shared = shared.clone();
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Value<'js> {
                let args = args
                    .iter()
                    .map(|value| json_of(&ctx, value))
                    .collect::<Vec<_>>()
                    .join(", ");
                shared.borrow_mut().intents.push(Intent {
                    namespace,
                    call,
                    args,
                });
                Value::new_null(ctx)
            },
        )
    }

    /// `ubiq.project`: the front project and the catalogue, both read-only.
    fn project<'js>(ctx: &Ctx<'js>, facts: &HostFacts) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("info", fact(ctx, HostFacts::or_null(&facts.project))?)?;
        object.set("list", fact(ctx, HostFacts::or_null(&facts.projects))?)?;
        // `null` rather than `undefined` when there is no project: the reference says this answers
        // "the front project's folder, or null", and rquickjs turns a `None` into `undefined`,
        // which is a different thing for a script written against that sentence.
        let root = facts
            .project
            .get("root")
            .and_then(|root| root.as_str())
            .map(|root| root.to_string());
        object.set(
            "root",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>| match &root {
                Some(root) => rquickjs::String::from_str(ctx.clone(), root)
                    .map(Value::from)
                    .unwrap_or_else(|_| Value::new_null(ctx)),
                None => Value::new_null(ctx),
            })?,
        )?;
        Ok(object)
    }

    /// `ubiq.tasks`: the snapshot, a lookup over it, and two intents.
    fn tasks<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        facts: &HostFacts,
    ) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("list", fact(ctx, HostFacts::or_null(&facts.tasks))?)?;

        // `get` is resolved in Rust rather than left to the script, so the id is matched the same
        // way whatever the task's shape: whichever of the two id fields the interface filled in.
        let all = facts.tasks.clone();
        object.set(
            "get",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: String| {
                let found = all.as_array().into_iter().flatten().find(|task| {
                    ["id", "taskId"]
                        .iter()
                        .filter_map(|key| task.get(*key).and_then(|v| v.as_str()))
                        .any(|value| value == id)
                });
                match found {
                    Some(task) => ctx
                        .json_parse(task.to_string())
                        .unwrap_or_else(|_| Value::new_null(ctx.clone())),
                    None => Value::new_null(ctx),
                }
            })?,
        )?;
        object.set("create", intent(ctx, shared, "tasks", "create")?)?;
        object.set("cancel", intent(ctx, shared, "tasks", "cancel")?)?;
        Ok(object)
    }

    /// `ubiq.ai`: what can be reached, and the inference that is recorded rather than run.
    fn ai<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        facts: &HostFacts,
    ) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("models", fact(ctx, HostFacts::or_null(&facts.models))?)?;
        let reachable = facts
            .models
            .as_array()
            .map(|models| !models.is_empty())
            .unwrap_or(false);
        object.set("available", Function::new(ctx.clone(), move || reachable)?)?;
        object.set("ask", intent(ctx, shared, "ai", "ask")?)?;
        Ok(object)
    }

    /// `ubiq.search`: the last query's results, filtered, and the next one as an intent.
    fn search<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        facts: &HostFacts,
    ) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("last", fact(ctx, HostFacts::or_null(&facts.search))?)?;

        // The filter is in Rust because the snapshot is: a script asking for ".rs" wants the paths
        // out of the last search, not the whole envelope to pick through.
        let results = facts
            .search
            .get("results")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        object.set(
            "files",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, pattern: Opt<String>| -> Value<'js> {
                    let needle = pattern.0.unwrap_or_default();
                    let hits: Vec<&serde_json::Value> = results
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|hit| {
                            needle.is_empty()
                                || hit
                                    .get("path")
                                    .and_then(|path| path.as_str())
                                    .map(|path| path.contains(&needle))
                                    .unwrap_or(false)
                        })
                        .collect();
                    ctx.json_parse(
                        serde_json::Value::from_iter(hits.into_iter().cloned()).to_string(),
                    )
                    .unwrap_or_else(|_| Value::new_null(ctx.clone()))
                },
            )?,
        )?;
        object.set("text", intent(ctx, shared, "search", "text")?)?;
        Ok(object)
    }

    /// `ubiq.sink`: the panel a script declares, the event it is being replayed for, and the
    /// withdrawal of both.
    fn sink<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        event: Option<&ScriptEvent>,
        handler: &Rc<RefCell<Option<Function<'js>>>>,
    ) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;

        let declared = shared.clone();
        let slot = handler.clone();
        object.set(
            "ui",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, payload: Value<'js>, callback: Opt<Function<'js>>| {
                    // A string payload is taken as the JSON it claims to be; anything else is
                    // stringified. Both are how the A2UI page's editor would have held it.
                    let json = match payload.as_string() {
                        Some(string) => string.to_string().unwrap_or_default(),
                        None => json_of(&ctx, &payload),
                    };
                    let has_handler = callback.0.is_some();
                    if let Some(function) = callback.0 {
                        *slot.borrow_mut() = Some(function);
                    }
                    declared.borrow_mut().ui = Some(ScriptUi {
                        payload: json,
                        handler: has_handler || slot.borrow().is_some(),
                    });
                },
            )?,
        )?;

        let replay = event.map(|event| event.as_json());
        object.set(
            "event",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>| match &replay {
                Some(json) => ctx
                    .json_parse(json.clone())
                    .unwrap_or_else(|_| Value::new_null(ctx.clone())),
                None => Value::new_null(ctx),
            })?,
        )?;

        let withdrawn = shared.clone();
        let slot = handler.clone();
        object.set(
            "clear",
            Function::new(ctx.clone(), move || {
                withdrawn.borrow_mut().ui = None;
                *slot.borrow_mut() = None;
            })?,
        )?;

        Ok(object)
    }

    /// One console instruction: every argument formatted, joined with a space, filed at `level`.
    fn writer<'js>(
        ctx: &Ctx<'js>,
        shared: &Shared,
        level: ConsoleLevel,
    ) -> rquickjs::Result<Function<'js>> {
        let shared = shared.clone();
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let text = args
                .iter()
                .map(|value| format_value(&ctx, value))
                .collect::<Vec<_>>()
                .join(" ");
            shared.borrow_mut().lines.push(ConsoleLine { level, text });
        })
    }

    /// A value as JSON, for an intent's argument list. A value JSON has no word for is named
    /// rather than dropped, so an intent never silently loses an argument.
    fn json_of<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
        match ctx.json_stringify(value.clone()) {
            Ok(Some(json)) => json
                .to_string()
                .unwrap_or_else(|_| "\"[object]\"".to_string()),
            Ok(None) => "undefined".to_string(),
            Err(_) => {
                let _ = ctx.catch();
                "\"[object]\"".to_string()
            }
        }
    }

    /// A value as the console should print it: a string bare, a primitive as JS writes it, anything
    /// else through `JSON.stringify` — and `[object]` when that throws, which a cycle does.
    fn format_value<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
        if let Some(string) = value.as_string() {
            return string
                .to_string()
                .unwrap_or_else(|_| "[string]".to_string());
        }
        if value.is_undefined() {
            return "undefined".to_string();
        }
        if value.is_null() {
            return "null".to_string();
        }
        if let Some(flag) = value.as_bool() {
            return flag.to_string();
        }
        if let Some(int) = value.as_int() {
            return int.to_string();
        }
        if let Some(float) = value.as_float() {
            return format_number(float);
        }
        if value.is_symbol() {
            return "[symbol]".to_string();
        }
        if value.is_function() {
            return "[function]".to_string();
        }

        match ctx.json_stringify(value.clone()) {
            Ok(Some(json)) => json.to_string().unwrap_or_else(|_| "[object]".to_string()),
            Ok(None) => "undefined".to_string(),
            Err(_) => {
                // `JSON.stringify` threw — a cycle, or a `toJSON` of the script's own. Take the
                // pending exception out of the context so it cannot surface as the run's error.
                let _ = ctx.catch();
                "[object]".to_string()
            }
        }
    }

    /// A double as JS prints it: no trailing `.0`, and the three values JSON has no word for.
    fn format_number(float: f64) -> String {
        if float.is_nan() {
            "NaN".to_string()
        } else if float.is_infinite() {
            if float.is_sign_negative() {
                "-Infinity".to_string()
            } else {
                "Infinity".to_string()
            }
        } else if float.fract() == 0.0 && float.abs() < 1e21 {
            format!("{float:.0}")
        } else {
            float.to_string()
        }
    }

    /// What went wrong, in one string. A run the interrupt handler stopped says so — QuickJS
    /// reports it as a bare `InternalError: interrupted`, which tells the reader nothing.
    fn describe_error(
        ctx: &Ctx<'_>,
        error: &rquickjs::Error,
        started: Instant,
        block_ms: u64,
    ) -> String {
        if started.elapsed().as_millis() as u64 >= block_ms {
            let _ = ctx.catch();
            return format!(
                "the script was stopped: a single step of execution ran past the {block_ms} ms \
                 block limit"
            );
        }

        if !error.is_exception() {
            return error.to_string();
        }

        let thrown = ctx.catch();
        let Some(exception) = thrown.as_exception() else {
            return format_value(ctx, &thrown);
        };

        let message = exception
            .message()
            .unwrap_or_else(|| "uncaught exception".to_string());
        match exception.stack() {
            Some(stack) if !stack.trim().is_empty() => format!("{message}\n{}", stack.trim_end()),
            _ => message,
        }
    }
}

/// The oxc half: parsing either dialect, and compiling TypeScript down to the JavaScript QuickJS
/// actually runs. Neither needs the interpreter, so both exist in every build.
///
/// oxc is a real front end rather than a grammar: it reports what a compiler reports, at the span
/// it found it, and its transformer lowers the whole TypeScript surface — enums and namespaces
/// included — instead of the erasable subset a syntax tree can be edited down to.
mod front {
    use std::path::Path;

    use oxc::allocator::Allocator;
    use oxc::codegen::Codegen;
    use oxc::diagnostics::OxcDiagnostic;
    use oxc::parser::Parser;
    use oxc::semantic::SemanticBuilder;
    use oxc::span::SourceType;
    use oxc::transformer::{TransformOptions, Transformer};

    use super::{Buffer, Diagnostic, Dialect, OxcOptions, SyntaxReport};

    /// The shape the page's own `ctx.eval` gives a program: a script, not a module — nothing here
    /// imports, and there is no loader behind the context to import with.
    fn source_type(dialect: Dialect, options: &OxcOptions) -> SourceType {
        let base = SourceType::default()
            .with_module(false)
            .with_jsx(options.jsx);
        match dialect {
            Dialect::Js => base,
            Dialect::Ts => base.with_typescript(true),
        }
    }

    /// What the transformer lowers to. An unknown target is not a failure the page can act on, so
    /// it falls back to lowering nothing but the types.
    fn transform_options(options: &OxcOptions) -> TransformOptions {
        TransformOptions::from_target(options.target.label()).unwrap_or_default()
    }

    /// Compile a program to the JavaScript QuickJS runs. An `Err` is the first thing the parser
    /// or the transformer refused, in the same one-line form a console prints.
    pub fn transpile(
        source: &str,
        dialect: Dialect,
        options: &OxcOptions,
    ) -> Result<String, String> {
        let allocator = Allocator::default();
        let mut parsed = Parser::new(&allocator, source, source_type(dialect, options)).parse();
        if let Some(first) = parsed.diagnostics.first() {
            return Err(describe(source, first));
        }

        // The transformer needs the scopes and symbols the semantic pass builds; it is the same
        // program, walked once more, and nothing here checks types. `with_enum_eval` is what
        // makes an enum's constant folding available — the transformer panics without it.
        let scoping = SemanticBuilder::new()
            .with_enum_eval(true)
            .build(&parsed.program)
            .semantic
            .into_scoping();
        let transformed = Transformer::new(
            &allocator,
            Path::new(file_name(dialect)),
            &transform_options(options),
        )
        .build_with_scoping(scoping, &mut parsed.program);
        if let Some(first) = transformed.diagnostics.first() {
            return Err(describe(source, first));
        }

        Ok(Codegen::new().build(&parsed.program).code)
    }

    /// The name the transformer is told the program has. It reads the extension to decide what a
    /// file is, so a `.ts` name is what makes it lower TypeScript at all.
    fn file_name(dialect: Dialect) -> &'static str {
        match dialect {
            Dialect::Js => "script.jsx",
            Dialect::Ts => "script.tsx",
        }
    }

    /// Syntax-check both of the page's buffers without running either, so a broken scratchpad says
    /// *where* before a run ever gets to stop it. TypeScript is transformed as well as parsed,
    /// because a program this page cannot lower is a problem the page has to name.
    pub fn validate(
        prelude: &str,
        source: &str,
        dialect: Dialect,
        options: &OxcOptions,
    ) -> SyntaxReport {
        let started = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut lines_checked = source.lines().count();

        if options.check_prelude && !prelude.trim().is_empty() {
            lines_checked += prelude.lines().count();
            errors.extend(check(prelude, Buffer::Prelude, dialect, options));
        }
        errors.extend(check(source, Buffer::Script, dialect, options));
        errors.truncate(options.max_errors);

        SyntaxReport {
            dialect,
            errors,
            lines_checked,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }

    /// One buffer, parsed and — when something will be lowered — transformed.
    fn check(
        source: &str,
        buffer: Buffer,
        dialect: Dialect,
        options: &OxcOptions,
    ) -> Vec<Diagnostic> {
        let allocator = Allocator::default();
        let mut parsed = Parser::new(&allocator, source, source_type(dialect, options)).parse();
        let mut errors: Vec<Diagnostic> = parsed
            .diagnostics
            .iter()
            .map(|error| to_diagnostic(source, buffer, error))
            .collect();

        let transforms = dialect == Dialect::Ts || options.transform_js;
        if errors.is_empty() && transforms {
            let scoping = SemanticBuilder::new()
                .with_enum_eval(true)
                .build(&parsed.program)
                .semantic
                .into_scoping();
            let transformed = Transformer::new(
                &allocator,
                Path::new(file_name(dialect)),
                &transform_options(options),
            )
            .build_with_scoping(scoping, &mut parsed.program);
            errors.extend(
                transformed
                    .diagnostics
                    .iter()
                    .map(|error| to_diagnostic(source, buffer, error)),
            );
        }

        errors
    }

    /// One oxc diagnostic, placed in the source the way the page's gutter wants it.
    fn to_diagnostic(source: &str, buffer: Buffer, error: &OxcDiagnostic) -> Diagnostic {
        let span = error
            .labels
            .first()
            .map(|label| (label.offset(), label.len()));
        let (line, column) = span.map_or((1, 1), |(offset, _)| position(source, offset));
        let snippet = span
            .and_then(|(offset, len)| source.get(offset as usize..(offset + len) as usize))
            .unwrap_or_default()
            .trim()
            .chars()
            .take(72)
            .collect();

        Diagnostic {
            buffer,
            line,
            column,
            message: error.message.to_string(),
            snippet,
        }
    }

    /// The same diagnostic as one line, for the callers that carry a `String` and not a report.
    fn describe(source: &str, error: &OxcDiagnostic) -> String {
        let diagnostic = to_diagnostic(source, Buffer::Script, error);
        format!(
            "{} (line {}, column {})",
            diagnostic.message, diagnostic.line, diagnostic.column
        )
    }

    /// A byte offset as the one-based line and column a person counts.
    fn position(source: &str, offset: u32) -> (u32, u32) {
        let offset = (offset as usize).min(source.len());
        let before = &source[..offset];
        let line = before.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map_or(0, |last| last.chars().count()) as u32
            + 1;
        (line, column)
    }
}

/// Whether an interpreter is compiled in.
pub fn available() -> bool {
    cfg!(feature = "quickjs")
}

/// What is compiled in, for the page to name.
pub fn engine_name() -> &'static str {
    if cfg!(feature = "quickjs") {
        "QuickJS"
    } else {
        "none"
    }
}

/// Everything one call of [`eval`] is given: the two buffers, how to compile them, what the
/// interface knows, and the event being replayed, if any.
///
/// A struct rather than seven arguments because every caller fills most of it in from defaults —
/// a test wants a source and nothing else, and the page wants the whole of it.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub prelude: String,
    pub source: String,
    pub dialect: Dialect,
    pub options: OxcOptions,
    pub facts: HostFacts,
    /// The click being replayed. `None` on a plain Run.
    pub event: Option<ScriptEvent>,
}

impl Run {
    /// One program, in the default dialect, with nothing behind it. What a test starts from.
    pub fn new(prelude: &str, source: &str) -> Self {
        Self {
            prelude: prelude.to_string(),
            source: source.to_string(),
            ..Default::default()
        }
    }

    /// The same, in the dialect named.
    pub fn dialect(mut self, dialect: Dialect) -> Self {
        self.dialect = dialect;
        self
    }
}

/// Runs a [`Run`]'s prelude then its source in one fresh context, on a thread of its own, and
/// reports everything that happened within the [`BLOCK_LIMIT_MS`] / [`TOTAL_LIMIT_MS`] budgets.
#[cfg(feature = "quickjs")]
pub fn eval(run: Run) -> ScriptOutcome {
    run_isolated(run, BLOCK_LIMIT_MS, TOTAL_LIMIT_MS)
}

/// The run machinery with the two ceilings chosen by the caller, so a regression test can make
/// `while (true) {}` cost a quarter of a second instead of five. The page's Run button never
/// reaches in here — [`eval`] it.
#[cfg(feature = "quickjs")]
#[doc(hidden)]
pub fn eval_timed(run: Run, block_ms: u64, total_ms: u64) -> ScriptOutcome {
    run_isolated(run, block_ms, total_ms)
}

/// Runs a program — except that this build has no interpreter to run it in, so it reports that
/// and nothing else.
#[cfg(not(feature = "quickjs"))]
pub fn eval(_run: Run) -> ScriptOutcome {
    ScriptOutcome {
        error: Some(
            "this build of Ubiq has no script interpreter: it was built without the `quickjs` \
             feature"
                .to_string(),
        ),
        ..Default::default()
    }
}

/// Run one program on a thread the caller never blocks on:
///
/// - The interpreter — runtime, context, globals, evaluation — is built and dropped on its own
///   thread. Only the outcome crosses back, so a runaway script holds its own thread and not the
///   page's.
/// - The script's *own* loop is bounded by `block_ms` through the interrupt handler.
/// - The caller waits at most `total_ms`. A run the interrupt missed — an interpreter stuck in
///   something the bytecode loop does not visit — is abandoned, and its thread is left to finish
///   out its own `block_ms` and die. That is the documented escape hatch: the page always gets an
///   answer, and a dead interpreter costs nothing beyond the thread it is stuck on.
#[cfg(feature = "quickjs")]
fn run_isolated(run: Run, block_ms: u64, total_ms: u64) -> ScriptOutcome {
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::time::{Duration, Instant};

    let started = Instant::now();
    let (sender, receiver) = channel();
    let _ = std::thread::Builder::new()
        .name("ubiq-script".to_string())
        .spawn(move || {
            let outcome = engine::eval(
                &run.prelude,
                &run.source,
                run.dialect,
                run.options,
                &run.facts,
                run.event.as_ref(),
                block_ms,
            );
            let _ = sender.send(outcome);
        });

    match receiver.recv_timeout(Duration::from_millis(total_ms)) {
        Ok(outcome) => ScriptOutcome {
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..outcome
        },
        Err(RecvTimeoutError::Timeout) => ScriptOutcome {
            error: Some(format!(
                "the run did not finish within the {total_ms} ms total time limit"
            )),
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..Default::default()
        },
        Err(RecvTimeoutError::Disconnected) => ScriptOutcome {
            error: Some(
                "the interpreter worker stopped without answering — it panicked".to_string(),
            ),
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..Default::default()
        },
    }
}

/// Compile a program to the JavaScript the interpreter runs. An `Err` names what oxc refused.
/// Available with or without the interpreter, since it is a compile not a run.
pub fn transpile(source: &str, dialect: Dialect, options: &OxcOptions) -> Result<String, String> {
    front::transpile(source, dialect, options)
}

/// Syntax-check both buffers without running them. See [`Dialect`] for what each dialect means
/// here, and [`OxcOptions`] for what the page's settings panel changes about it.
pub fn validate(
    prelude: &str,
    source: &str,
    dialect: Dialect,
    options: &OxcOptions,
) -> SyntaxReport {
    front::validate(prelude, source, dialect, options)
}
