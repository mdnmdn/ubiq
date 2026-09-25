---
id: feat-workbench-sink
title: Sink mode — the kitchen sink
kind: feature
status: draft
summary: The rail's Sink mode — the application's own test bench, twelve pages of fixtures with nothing behind them — a buffer, one page per special viewer, the style reference, the file picker in each shape a screen can ask for, the two settings layouts, a live conversation beside its bus traffic, the A2UI surface, the script scratchpad and the teamsim testbed.
read_when: you are changing the kitchen sink's pages or fixtures, the A2UI renderer, the script scratchpad, the teamsim testbed, or looking for a surface to try a primitive on
updated: 2026-09-25
verified: 2026-09-25
code_anchors: [crates/ubiq/src/state/sink.rs, crates/ubiq/src/app/sink.rs, crates/ubiq/src/ui/sink/mod.rs, crates/ubiq/src/ui/sink/docs.rs, crates/ubiq/src/ui/sink/style.rs, crates/ubiq/src/ui/sink/files.rs, crates/ubiq/src/ui/sink/settings.rs, crates/ubiq/src/ui/sink/script.rs, crates/ubiq/src/ui/sink/teamsim.rs, crates/ubiq/src/ui/sink/a2ui.rs, crates/ubiq/src/ui/sink/project.rs, crates/ubiq/src/state/script.rs, crates/ubiq/src/state/teamsim.rs, crates/ubiq/src/state/a2ui.rs, crates/ubiq/src/state/a2ui/value.rs, crates/ubiq/src/state/a2ui/eval.rs, crates/ubiq/src/state/a2ui/action.rs, crates/ubiq/src/state/a2ui/live.rs, crates/ubiq/src/state/a2ui/svg.rs, crates/ubiq/src/state/a2ui/path.rs, crates/ubiq/src/state/a2ui/ubiq-catalog.json, crates/ubiq/src/ui/a2ui.rs, crates/ubiq/src/ui/a2ui/registry.rs, crates/ubiq/src/ui/a2ui/svg.rs, crates/ubiq/src/ui/a2ui/path.rs, crates/ubiq/src/ui/kit/blocks.rs, crates/ubiq/src/ui/kit/menu.rs, _tools/teamsim/FORMAT.md, crates/ubiq/tests/a2ui.rs, crates/ubiq/tests/script.rs, crates/ubiq/tests/sink.rs]
depends_on: [feat-workbench, tech-ui]
review_cycle: monthly
---

# Sink mode — the kitchen sink

## Purpose

The kitchen sink is the one screen with nothing behind it: every page is a fixture, it asks the
host for nothing, and it looks the same in every window with or without a project. It exists so
that a primitive, a viewer, a settings layout, an agent-authored surface or a graph arrangement
can be looked at and judged without arranging a project to produce one.

## Behaviour

**The kitchen sink is the application's own test bench, and the one screen with nothing behind it.**
It is under `APP` because it is about Ubiq rather than about a folder: it opens on a first run with an
empty catalogue, it asks the host for nothing, and it looks the same in every window. Twelve pages,
selected by a strip along the top — the plain buffer, one per special viewer, the style reference,
the file picker, the two settings layouts composed from the kit, one live conversation beside the
bus traffic behind it, the A2UI surface, the script scratchpad, and the teamsim testbed the graph's
arrangements are judged in.

**Its documents are fixtures, drawn by the viewer their name implies, and a fixture is not a file.**
A page's document is a name and a constant, and the name carries an extension, so the viewer and the
highlighter are picked by exactly the rule a real path goes through rather than by a second table
that could disagree with it. The Markdown fixture carries a fence of each diagram kind, which is how
the fence renderers are exercised without opening a file, and what a viewer is handed is the
**buffer's** current text rather than the constant, so typing in the source half of a split redraws
the drawn half. But a fixture has no path, no version and nothing to save, so no tab, no dirty mark
and no save state is drawn for one: all the sink keeps per document is which of its viewer's layouts
is on screen, toggled by the same three pills a file panel offers.

**The style reference draws every token and every primitive, and it is where one with no other call
site gets one.** Each specimen carries the name a call site reaches it by, so a token whose value is
wrong in one palette, a control whose off state reads as absent, or a surface whose coloured edge
floats inside its container shows up here before it shows up on a screen. Its controls are wired to
real state, because a control that cannot hold a value is not being tested — one value drives the
stepper, the slider, the meter and the ring — and nothing they hold means anything. The typography specimen is
the same idea one axis over: five roles down, the three surface families across, each run at what
`theme::font` answers for it, so a base moved in the interface prefs is looked at rather than
computed.

**Its modals are the window's only ones, and nothing behind them happens.** Three shapes — a
question, a form, and something irreversible — told apart by what their edge says and by the colour
of the confirming button. Both buttons dismiss and neither claims anything: a fixture that pretended
to close a pane would be the one thing a screen with nothing behind it must not draw. The shape a
modal is drawn in belongs to the UI-and-design document, linked below, with every other surface's.

**The A2UI page draws a surface an agent could send, from the JSON beside it.** A2UI is a protocol
Ubiq does not own — `_docs/references/a2ui-protocol.md` is its wire and
`_docs/references/a2ui-catalog.md` its component set — and this page is where the renderer for it is
looked at before any agent drives one. A picker of seven example payloads sits over an editor on the
right; the left is what `state::a2ui::parse` made of whatever that editor holds, re-read when the
buffer changes rather than on every frame. Four of the seven are upstream conformance fixtures and
three are written here, and the page says which is which, because whether the renderer handles real
agent output turns on it.

**A payload that stops parsing puts the reason above the last surface that did, rather than in place
of it.** The banner is the whole of the report, and the drawing under it stays: dropping the surface
would drop every field buffer with it and throw away what the reader typed into the drawing, which
is exactly what a half-finished keystroke in the editor must not cost. A payload that *does* parse
resets the data model instead of merging into it, because the payload carries one.

**The surface's inputs are live, and every write lands in the data model.** A2UI carries values in a
data model addressed by JSON Pointer, and `state/a2ui/value.rs` is that pointer: it reads, it
writes, it creates the objects and arrays a path passes through, `-` appends to an array and a null
removes. `state/a2ui/eval.rs` is what a bound property resolves through — a literal is itself, a
`{"path": …}` is a lookup, and a `{"call": …}` is one of the catalog's fourteen functions, plus the
reserved `@index`. Every keystroke, tick, choice and nudge funnels through the single mutator in
`state/a2ui/live.rs`, so every other component bound to that pointer redraws with it.

**A text input costs an entity; nothing else does.** One `Entity<InputState>` per text field, in a
map keyed by component id, allocated when the payload is re-parsed and never during a render, with
its subscription parked in the same map — so clearing the map is the whole teardown. A checkbox, a
choice and a slider are clicks and hold no entity. Tabs and modals are keyed by component id the
same way, because where a reader has navigated to is not a value the agent is owed.

**A templated list expands once per item the pointer finds, and a relative path resolves against
that item.** A `TextField` bound to `qty` inside a row over `/items` writes `/items/2/qty` in the
third row, so a template is as live as the components it repeats rather than a drawing made once.

**A button is checked before it is reported, and a function call is reported rather than
performed.** A click evaluates every `checks` rule in the surface; a failing one blocks the action
and its message is drawn under the input that refused it. Otherwise the event's `context` is
resolved against the component's own scope and `state/a2ui/action.rs` builds the envelope the agent
would receive — the action's name, the surface and source ids, a timestamp, the resolved context and
the optional `userMessage`, with the whole data model as a top-level sibling when the surface asked
for one. An action carrying a `functionCall` is reported and never carried out: `openUrl` opens
nothing, which is `D115`.

**The component set is a registry rather than a closed enum, and Ubiq ships a catalog of its own.**
`ui/a2ui/registry.rs` is a table of name against drawing function; a component in neither the basic
catalog nor that table keeps its props and draws a placeholder. What the table is held to is
`crates/ubiq/src/state/a2ui/ubiq-catalog.json`, embedded in the binary, which extends the basic
catalog with exactly one component — `Svg` — and which `crates/ubiq/tests/a2ui.rs` checks the
registry against name for name. `D114` is why there are two sources of truth and one test between
them.

**Two routes draw vector art, and which one a payload takes is a security question.** `Svg` carries
markup, so `state/a2ui/svg.rs` runs an allow-list scanner over it before anything is drawn —
refusing script, foreign objects, references out of the document, any event attribute, external
`url()`, data URIs, doctypes and entities, anything over the size or element ceiling, and anything
with no usable `viewBox` — and what survives is handed to GPUI as an image sized from that
`viewBox`, with `currentColor` taking the text token. A refusal is **drawn**, never blank, and the
Ubiq-catalog example carries one on purpose so a reader sees what a refusal looks like. The basic
catalog's `Icon` takes the other route: `state/a2ui/path.rs` parses its `svgPath` and
`ui/a2ui/path.rs` paints it on a canvas. That route needs no scanner at all, which is the reason
both exist — a path string can express geometry and nothing else.

**Under the editor sit the two things a surface is judged by.** A Data model tab draws the live
model as it is typed into, and an Actions tab stacks the envelopes newest first with a control to
clear them, so what a click would have sent is read on the page rather than inferred from it. Three
components still draw a placeholder rather than themselves: GPUI has no video or audio element, so
`Video` and `AudioPlayer` report their URL, and `Image` does the same, because a drawn surface
fetches nothing.

**The script page runs JavaScript through the interpreter compiled into the interface, and holds
nothing between runs.** Two editors sit over a console: the program, and a shorter **prelude** of
custom instructions that runs first in the same context, so a helper a reader leans on is written
once rather than re-typed into every experiment. Nothing runs on a keystroke — Run is the only
trigger, because a scratchpad evaluated while it is typed evaluates half-typed programs — and what
comes back is the console lines in the order the script wrote them, coloured by level, then every
intent the script recorded (in the warning colour), then the completion value or the exception,
then a footer with the milliseconds the run took and whether a panel was declared. Clear empties
the console. `console.log`, `.info`, `.warn` and `.error` are captured; a `ubiq` global carries five
core instructions — `version()`, `platform()`, `now()`, `log(...args)` and `echo(value)` — plus five
namespaces documented in `script::API_DOC`, rendered as Markdown under the console: `ubiq.project`
(`info`/`root`/`list`), `ubiq.tasks` (`list`/`get`/`create`/`cancel`), `ubiq.ai`
(`available`/`models`/`ask`), `ubiq.search` (`last`/`files`/`text`), and `ubiq.sink`
(`ui`/`event`/`clear`).

**A read answers from a snapshot; a write is an intent, recorded and never performed.**
`AppState::script_facts` serialises everything a script may read — the front project, the project
catalogue, the tasks in flight, the reachable models, the last search — as JSON, on the window's own
thread, before the interpreter exists. `ubiq.project`, `ubiq.tasks.list`/`get`, `ubiq.ai.models`/
`available` and `ubiq.search.last`/`files` answer from that snapshot and nothing else; the
interpreter cannot re-enter the interface and the interface cannot be blocked by a script.
`ubiq.tasks.create`, `ubiq.tasks.cancel`, `ubiq.ai.ask` and `ubiq.search.text` are **intents**: the
call is recorded, with its arguments, and nothing it names is carried out — the same rule `D115`
puts on an A2UI local call. The console draws every intent in the warning colour, as an account of
what was asked for and refused.

**`ubiq.sink.ui(payload, handler)` declares a panel, and a click on it replays the script from the
top.** The payload is exactly what the A2UI page's own editor takes, and the page draws it in its
right half when the switch there is set to *Panel* rather than *Docs* — a Panel pill carries a dot
whenever the last run declared one. Because nothing survives a run, there is no closure left to call
on a click: `fire_script_a2ui_action` does the A2UI page's own envelope-and-blocking-check work and
then re-runs the whole script with a `ScriptEvent` in hand, and the handler the second declaration
registers is called once the script has finished evaluating. `ubiq.sink.event()` answers that event,
or `null` on a plain Run, which is what lets a script branch on whether it is being replayed and stay
idempotent. A declaration made with no handler leaves a note in the panel's log instead of
replaying.

**TypeScript is accepted whole, compiled to JavaScript, and never type-checked.** A source in the
TypeScript dialect is parsed and lowered by `oxc` before the interpreter sees a character of it, so
an enum, a namespace and a generic call argument are turned into the JavaScript they mean and they
run — the page refuses no construct the language has. A syntax error comes back as the compiler's
own message with the line and column it sits on, and names which of the two buffers it is in. What
no part of this does is check a type: `const x: string = 1` compiles and runs, because the page
erases types rather than judging them. `D128` is why the front end is a compiler, and what that
costs.

**An inline settings panel, disclosed under the chrome, points oxc at both buffers.** `OxcOptions`
carries whether JSX is allowed, which of five ECMAScript levels (`es2015` through `esnext`; oxc
refuses `es5` outright) the transformer lowers to, whether JavaScript goes through the transformer
too (TypeScript always does), whether the prelude is checked as well as the program, and how many
problems one report names. There is deliberately no "parse as a module" switch — oxc's parser
accepts `import`/`export` in either mode, so the pill would have had nothing behind it. Run and
Validate read the same options, so what Validate accepts is what Run compiles; lowering far below
the interpreter's own level can emit helper calls (`_classPrivateFieldInitSpec`) nothing here
defines, which is a property of the transformer and not a bug.

**Validate owns the console.** It clears the last run first and leaves exactly one thing: a verdict
line — green when both buffers parsed and compiled clean, naming the dialect, the lines checked and
the milliseconds it took; red naming how many problems there were — followed by one row per
`Diagnostic`, each naming which buffer it is in (`prelude` or `script`), so a line number never
points at the wrong editor.

**Every run is a fresh runtime on a worker thread, bounded, and nothing it does can panic the
window.** The page never waits on the interpreter — Run starts an evaluation that answers off the
window, its console lines and completion value landing when they land — and three ceilings bound a
runaway: a 5 second per-step interrupt that stops a single step of execution that ran past it (an
endless loop hits it, and the console says it was stopped), a 15 second ceiling on the whole run
past which the page stops waiting and abandons it, and a 16 MiB allocation cap past which the
interpreter throws out of memory. Every interpreter failure — a parse error, a throw, a stop —
arrives as the run's error rather than as an unwind. The page is drawn in every build: without the
`quickjs` feature there is no interpreter behind it and the page says so, which is what keeps a
feature-off build one page's contents short rather than one page short. `D127` is why the
interpreter is there at all, and what it costs; `D129` is the capability model — facts, intents and
a declared panel replayed rather than resumed.

**The teamsim page is where the graph's arrangements are judged, against a scenario a reader can
edit.** It is the one sink page whose fixture is a file format: `_tools/teamsim/scenarios/*.json`,
`ubiq.teamsim/1`, embedded with `include_str!` and described by `_tools/teamsim/FORMAT.md`. A
scenario is a whole graph — sessions, the tasks in them, the agents on those tasks, the delegates
inside an agent's fence, and the extra links between agents — and `state/teamsim.rs` projects one
into the very records the Teams canvas is drawn from, `WorkAgent`, `TaskRecord` and the ring count
per card, so `Layout::auto` and `Layout::place_new` place it. **Nothing on the page is a copy of the
packers**: a new `Algo` variant appears in the arrangement picker with no edit, because the picker
is `Algo::ALL` and the rows are `label()` and `hint()`.

**The scenario JSON is the source of truth, and the clipboard is the save file.** `+sub`, `Add
agent` and `Add task` edit the scenario and then place only what arrived, through `place_new` — the
same call an arriving card goes through in the product — so nothing already on the canvas moves, and
the metrics strip says how far it did move, which is the number an incremental arrangement is judged
on. `Tidy` is the one control that throws hand positions away and recomputes whole. Cards,
containers and delegates all move by drag, writing back through `Layout::place_task`,
`place_agent` and `place_sub`; `Copy JSON` writes those positions into the file's `positions` block
and loading one seeds the layout from it, so a saved scenario reproduces a picture and not only a
graph. `Paste JSON` reads one back and says inline what a file it cannot read is wrong about.

**The strip under the toolbar is the readability verdict**: the arrangement's bounding box, its
aspect, and the box over the scenario's own viewport in each axis — past one screen in warning, past
one and a half in danger. The arithmetic is `state/teamsim.rs`'s; the page reads it out.

**The settings pages are layouts, not a settings screen.** Application settings is a left nav of
kit rows — Appearance, Harnesses, Agent defaults, and the three quieter destinations — and a body
of the same controls the style reference already draws: `choice_pill` for a pinned theme or an
interface size, `check_box` for a boolean, `stepper` and `meter` for a number, `card` for a permission
mode, `Picker` for a dropdown, `slab` for a harness that opens. Project settings is that same
furniture in the shape of a dialog: a coloured left edge, a nav, a form. On the sink, Cancel puts
the fixture back and Save writes nothing, because the sink has no project behind it. Its Agent
definitions nav item is the one section the fixture can only half draw: the tick is real, and the
list under it is a project's own definitions, which a page with no project has none of — it says
so instead. Over the
workbench the same dialog is the create and edit surface: only General is enabled, the path is
immutable, Create sends `AddProject`, and Save sends `UpdateProject`. Its "Project data" row is
`choice_pill` too, and it is the one control that only Create can work: it names where the project
keeps its tasks and metadata, which is answered once and shown read-only afterwards. Its colour row is a strip of
swatches plus `kit::colour_picker` — saturation/value, a hue bar, and a `#RRGGBB` field — so a
custom colour is chosen rather than only indexed. The picker is the kit's, not this dialog's: it
reports a hue, a saturation and a value, and this form is the one that turns them into the
project's `custom` tint. A field that holds the keyboard is underlined on its bottom edge
as well as marked on the left.

**The picker page raises the file picker in each of the shapes a screen can ask for one.** The picker
takes a request — files or folders, one answer or several, the folder it is rooted at, a prefilter
like `*.md`, whether a single pick is final on the click or on the button, and whether it holds the
window or goes away on an outside click — and the page draws those seven fields as pill rows over a
button that raises the dialog out of them. The tree it opens over is a fixture like every other page's,
because the sink has no project behind it, and what the dialog handed back is printed under the
button: a cancelled dialog and one that answered with nothing are different answers, and the readout
says which it was. The dialog itself belongs to the window rather than to the page — exactly one may
be up, whichever screen asked — so the answer is routed back to whoever raised it rather than
returned.

**The dialog is worked from the keyboard, and the field keeps it the whole time.** It opens with the
focus in the filter — typing a name is the first thing a picker is for — and the keys that drive the
rows are bound against the field as well as against the dialog, so nothing has to be tabbed to.
`up` and `down` move a cursor bar through the rows and stop at the ends; `right` opens the folder the
cursor is on and then steps into it, `left` shuts it and then steps out to the folder holding it;
`enter` ticks the row where several may be chosen and *is the answer* where one was asked for;
`secondary-enter` — cmd on macOS, ctrl elsewhere — hands back what has been ticked; `escape` closes
the dialog with nothing. What the dialog has no answer for it hands back, which is how `left` and
`right` are the field's own caret keys again in the flat list.

**The cursor is not the selection, and the two are drawn apart.** The accent is what will come back;
the keyboard's bar is only where the next key lands, in `selected` with a `border_focus` edge. A row
that is both keeps the accent fill and takes the focus edge — what a dialog hands over outranks where
its cursor happens to be. The cursor follows the mouse too, so an arrow after a click carries on from
the row that was clicked, and a row it is moved onto is scrolled into view.

**Both arrangements are the same set, and which one is on screen is the user's.** The tree is the
folders that have been opened, indented, each file reporting its size; the list is every match under
the root, flat and sorted by name without case, each row saying which folder it came from. One filter
field sits over both and what was typed survives the toggle, because a user who cannot find something
in the tree switches to the list to look for the same thing. A filter finds rather than prunes: every
folder is walked while one is typed, and a folder with nothing matching under it drops out instead of
drawing as an empty row. A folders-only picker draws no files at all, and the prefilter never hides a
folder — a folder it hid would take the files under it with it.

## Implementation

Projects, the file tree, a file's bytes, the panes, the work, the chat's conversations and the
Git screen's refs and history all come from the host now; `state/sample.rs` and the constructors
that invented them are gone.

`state/sink.rs` is the kitchen sink's fixtures and the small state its pages carry, and it is the
one place a constant still stands in for what the host would send — deliberately: a test bench
with a project behind it would be testing the project rather than the bench. It holds four
documents as `&'static str`, each under the name that picks its viewer,
`SinkSection` for the page strip, `SinkState` for the layouts and the style reference's controls,
`SinkModal` for which of the three shapes is up, `SettingsDemo` for the application settings page,
`ProjectDemo` for the project settings dialog — its swatch, its custom `0xRRGGBB`, and the
HSV the picker is holding — and `ScriptDemo`, which carries the dialect, the picked
`SCRIPT_EXAMPLES` index, the `OxcOptions` the settings panel writes, the last `ScriptOutcome` and
`SyntaxReport`, which of the two `ScriptPane`s is forward, and the `Live` a declared panel is drawn
into. Nothing in it draws and nothing in it holds a
buffer, which is what lets `crates/ubiq/tests/sink.rs` hand every fixture to the parser or the
renderer that will draw it with no frame — so a fixture that stopped parsing fails the build instead
of drawing an error nobody looks at.

The buffers are the window's: `AppState` builds one `EditorState` per fixture in its constructor,
where there is a window to build one with, keyed by the document's key, plus `sink_input`,
`sink_textarea`, `sink_modal_input`, the script page's `script_buffer` and `script_prelude`
(seeded from whichever `SCRIPT_EXAMPLES` entry is picked, with no change subscription on either,
because the page reads them when Run or Validate is pressed and at no other moment), and the settings pages' own fields (`sink_search`, the
harness name, executable, prompt and env, the project name, description and colour hex). A fixture is a
constant, so that is the whole of their lifecycle — nothing arrives late, nothing is saved, and no
change subscription is needed because there is no baseline to compare against. `ui/sink/` is the
screen: `mod.rs` draws the page strip through `kit::tab_strip` and dispatches on the page,
`docs.rs` draws one fixture through `ui/viewer/` — every viewer reached rather than copied, which
is the whole point of the page — `style.rs` is the reference, `files.rs` is the picker page,
`settings.rs` is the application settings layout, `project.rs` is the project settings dialog and
`script.rs` is the scratchpad. The interpreter sits behind one facade,
`crates/ubiq/src/state/script.rs` — `available()`, `engine_name()`, and `eval(run: Run)` /
`eval_timed(run, block_ms, total_ms)` returning a `ScriptOutcome`, where `Run` carries the prelude,
the source, the `Dialect`, the `OxcOptions`, the `HostFacts` snapshot and the `ScriptEvent` being
replayed, if any — plus `validate(prelude, source, dialect, &OxcOptions)` returning a `SyntaxReport`
that checks both buffers, and `transpile(source, dialect, &OxcOptions)` returning the JavaScript a
program means, both `oxc` calls that need no interpreter and exist in every build. `AppState`
reaches it through `script_facts` (builds the `HostFacts` from the project snapshot, the
`WindowRegistry`, the project's `WorkProjection` tasks, the settings' `ai_providers`/`ai_models` and
the window's `search`), `run_sink_script` and `replay_sink_script` (both funnel into
`start_sink_script`, which spawns the evaluation off the window and lands the answer through
`ScriptDemo::seq`), `land_sink_script_panel` (parses a declared payload into `ScriptDemo::a2ui`),
`validate_sink_script`, `fire_script_a2ui_action` (the panel's own envelope-and-blocking-check work,
then a replay), and the option setters in `app/sink.rs` — and nothing else reaches it.
`crates/ubiq/tests/script.rs` drives that facade directly and is a target of the `quickjs` feature,
so a build without the interpreter compiles no test that needs one.
The modal is raised from `mod.rs` rather than from `style.rs`, because exactly one may be up and
where it is asked for is not where it is painted; the primitive is `kit::modal`, whose shape and
dismissal rules are the UI-and-design document's. The project settings page is not that modal: it
is a wider form. The sink draws it on the page so the whole of it can be looked at; the workbench
raises the same form over the window to create a project or edit the one on screen.

## Related docs

- [`workbench.md`](./workbench.md) — the rail, the dock and the panels this mode is drawn inside
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and primitives the style reference draws
- [`../references/a2ui-protocol.md`](../references/a2ui-protocol.md) — what the A2UI page renders
- [`../references/a2ui-catalog.md`](../references/a2ui-catalog.md) — the component vocabulary it draws from
- [`workbench-teams.md`](./workbench-teams.md) — the arrangements the teamsim page judges
- [`../backlog.md`](../backlog.md) — what this screen still lacks
