//! The script page: a JavaScript and TypeScript scratchpad over the embedded interpreter.
//!
//! **Two buffers and a console.** The prelude holds the custom instructions the reader wants
//! defined before every run; the program is what Run evaluates, in the same context, immediately
//! after it. Splitting them is not cosmetic — the prelude is the part kept across programs, and a
//! page with one buffer would make the reader re-type it on every experiment.
//!
//! **Nothing runs on a keystroke.** A scratchpad evaluated while it is being typed runs half-typed
//! programs, so the page runs on the button and holds the last answer until the next press.
//!
//! **oxc parses and compiles in front of QuickJS.** TypeScript is not stripped, it is lowered —
//! enums, namespaces and generic calls all run. Validate is oxc and nothing more, so it needs no
//! interpreter and works in a build with the `quickjs` feature off. Settings, beside the buttons,
//! opens what it is pointed at: JSX, the ECMAScript level it lowers to, whether JavaScript goes
//! through the transformer at all, and whether the prelude is checked too. Run and Validate read
//! the same settings, so what Validate accepts is what Run compiles.
//!
//! **Validate owns the console.** It clears it first and leaves exactly one thing behind: the
//! problems, or the line saying there are none. A verdict found at the bottom of the last run's
//! output is a verdict nobody reads.
//!
//! **The page never waits for the interpreter.** A run is off the UI thread, so the Run button
//! simply says it is busy until an answer lands.
//!
//! **The right half is either the reference or the script's own panel**, never both: a switch
//! chooses. The reference is Markdown, rendered by the component library's text view, so adding a
//! binding is adding a row to one string. The panel is whatever the script declared through
//! `ubiq.sink.ui` — and a click on it re-runs the script, because nothing survives a run.
//!
//! The page is drawn in every build. Without the `quickjs` feature there is no interpreter behind
//! it and the chrome says so plainly, because a page that vanishes with a feature flag is a page
//! nobody notices is missing.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_component::input::{Editor, EditorState};
use gpui_component::text::TextView;
use std::rc::Rc;

use crate::app::AppState;
use crate::state::script::{self, ConsoleLevel, Dialect, EsTarget, OxcOptions};
use crate::state::sink::ScriptPane;
use crate::state::workbench::MenuId;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::menu::{Picker, PickerStyle};
use crate::ui::kit::{
    choice_pill, ghost_button, mono, primary_button, section_label, slab, stepper, toggle_pill,
};
use crate::ui::{eid, handler, indexed};

/// The prelude's height: enough for the handful of lines a set of custom instructions is, and
/// fixed, so the program keeps the rest of the half.
const PRELUDE_HEIGHT: f32 = 120.;

/// The console's height. The result and the error slab sit inside it, so it is the taller pane.
const CONSOLE_HEIGHT: f32 = 260.;

/// How wide the example picker is allowed to be.
///
/// Bounded, and that is the whole point: a `PickerStyle::Field` takes the width it is given, and
/// in a flex row it took all of it and pushed the dialect toggle off the end. The row now has one
/// fixed-width control and one caption that may shrink, so nothing can crowd anything else out.
const PICKER_WIDTH: f32 = 260.;

pub fn render(app: &AppState, _window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            half(program(app, cx))
                .border_r_1()
                .border_color(theme::border()),
        )
        .child(half(right(app, cx)))
        .into_any_element()
}

/// The chrome, the prelude and the program: everything the reader edits, and the button that runs
/// it.
fn program(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(chrome(app, cx))
        .children(app.sink.script.settings_open.then(|| settings(app, cx)))
        .child(prelude(app))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(caption(
                    "Script — evaluated after the prelude, in the same context",
                ))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .child(buffer(&app.script_buffer)),
                ),
        )
        .into_any_element()
}

/// Three rows: the example picker, the buttons beside the dialect toggle, and what is behind both.
///
/// The dialect toggle sits on the button row rather than beside the picker because that is where
/// it belongs by meaning as well as by width — it is a property of the run, like Run and Validate,
/// and not a property of the example.
fn chrome(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let picked = app.sink.script.example;
    let names: Vec<&'static str> = crate::state::sink::SCRIPT_EXAMPLES
        .iter()
        .map(|example| example.name)
        .collect();
    let current = crate::state::sink::SCRIPT_EXAMPLES.get(picked);

    let engine = match script::available() {
        true => format!(
            "{} \u{00b7} oxc {}",
            script::engine_name(),
            app.sink.script.options.summary()
        ),
        false => format!(
            "Engine: {} — this build has no `quickjs` feature, so Run only says so.",
            script::engine_name()
        ),
    };

    let run_label = if app.sink.script.running {
        "Running\u{2026}"
    } else {
        "Run"
    };

    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .p_2()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div().flex_none().w(px(PICKER_WIDTH)).child(
                        Picker::new(
                            "script-example",
                            current.map(|example| example.name).unwrap_or_default(),
                        )
                        .items(names)
                        .selected(picked)
                        .style(PickerStyle::Field)
                        .open(app.workbench.open_menu == Some(MenuId::SinkScript))
                        .on_toggle(handler(&cx.entity(), |this, _, cx| {
                            this.open_menu(MenuId::SinkScript, cx)
                        }))
                        .on_pick(indexed(&cx.entity(), |this, index, window, cx| {
                            this.pick_sink_script_example(index, window, cx)
                        }))
                        .on_dismiss(handler(&cx.entity(), |this, _, cx| this.close_menu(cx))),
                    ),
                )
                .children(current.map(|example| {
                    mono(example.origin, theme::text_faint())
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                        .into_any_element()
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(primary_button(
                    "script-run",
                    None,
                    run_label,
                    handler_click(cx, |this, cx| this.run_sink_script(cx)),
                ))
                .child(ghost_button(
                    "script-validate",
                    None,
                    "Validate",
                    handler_click(cx, |this, cx| this.validate_sink_script(cx)),
                ))
                .child(ghost_button(
                    "script-clear",
                    None,
                    "Clear",
                    handler_click(cx, |this, cx| this.clear_sink_script(cx)),
                ))
                .child(div().flex_1().min_w(px(0.)))
                .child(div().flex().flex_none().items_center().gap_1().children(
                    Dialect::all().iter().map(|dialect| {
                        choice_pill(
                            eid("script-dialect", dialect.short()),
                            dialect.short(),
                            app.sink.script.dialect == *dialect,
                            handler_click(cx, move |this, cx| {
                                this.set_sink_script_dialect(*dialect, cx)
                            }),
                        )
                        .into_any_element()
                    }),
                ))
                .child(ghost_button(
                    "script-settings",
                    None,
                    if app.sink.script.settings_open {
                        "Settings \u{25be}"
                    } else {
                        "Settings \u{25b8}"
                    },
                    handler_click(cx, |this, cx| this.toggle_sink_script_settings(cx)),
                )),
        )
        .child(
            mono(
                engine,
                match script::available() {
                    true => theme::text_faint(),
                    false => theme::warning(),
                },
            )
            .text_size(theme::font(Family::Chrome, Role::Micro)),
        )
        .into_any_element()
}

/// The oxc settings: what the front end is pointed at, disclosed under the chrome.
///
/// Inline rather than a modal because these are dials a reader turns *while* reading the report
/// they change — lowering the target and pressing Validate again is one motion, and a dialog that
/// covered the program would make it two.
fn settings(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let options = &app.sink.script.options;

    // One pill per switch, each naming what it turns on rather than what the field is called:
    // "module" alone reads as a noun, and every one of these is a verb.
    let switches: Vec<AnyElement> = [
        (
            "JSX",
            options.jsx,
            (|o: &mut OxcOptions| &mut o.jsx) as fn(&mut OxcOptions) -> &mut bool,
        ),
        (
            "lower JS too",
            options.transform_js,
            |o: &mut OxcOptions| &mut o.transform_js,
        ),
        (
            "check prelude",
            options.check_prelude,
            |o: &mut OxcOptions| &mut o.check_prelude,
        ),
    ]
    .into_iter()
    .map(|(label, on, pick)| {
        toggle_pill(
            eid("script-option", label),
            label,
            theme::accent(),
            on,
            handler_click(cx, move |this, cx| this.toggle_sink_script_option(pick, cx)),
        )
        .into_any_element()
    })
    .collect();

    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1p5()
        .p_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(section_label(
            "oxc — the front end in front of the interpreter",
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(switches),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    mono("target", theme::text_muted())
                        .text_size(theme::font(Family::Chrome, Role::Micro)),
                )
                .children(EsTarget::all().iter().map(|target| {
                    choice_pill(
                        eid("script-target", target.label()),
                        target.label(),
                        options.target == *target,
                        handler_click(cx, move |this, cx| this.set_sink_script_target(*target, cx)),
                    )
                    .into_any_element()
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(stepper(
                    "script-max-errors",
                    format!("{} errors reported", options.max_errors),
                    handler_click(cx, |this, cx| this.step_sink_script_max_errors(-5, cx)),
                    handler_click(cx, |this, cx| this.step_sink_script_max_errors(5, cx)),
                ))
                .child(div().flex_1().min_w(px(0.)))
                .child(ghost_button(
                    "script-options-reset",
                    None,
                    "Reset",
                    handler_click(cx, |this, cx| this.reset_sink_script_options(cx)),
                )),
        )
        .into_any_element()
}

/// The custom instructions, at a fixed height: they are a preamble, not the program.
fn prelude(app: &AppState) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .border_b_1()
        .border_color(theme::border())
        .child(caption("Prelude — runs first, before every script"))
        .child(
            div()
                .h(px(PRELUDE_HEIGHT))
                .child(buffer(&app.script_prelude)),
        )
        .into_any_element()
}

/// The console over what the switch chose: the reference, or the script's own panel.
fn right(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .h(px(CONSOLE_HEIGHT))
                .border_b_1()
                .border_color(theme::border())
                .child(caption("Console"))
                .child(console(app)),
        )
        .child(lower(app, cx))
        .into_any_element()
}

/// One row per printed line, then every intent, then the result or the error, then how long it
/// took — or, when Validate has spoken, its verdict and nothing else.
fn console(app: &AppState) -> AnyElement {
    let shell = || {
        div()
            .id("script-console")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .p_2()
            .gap_0p5()
            .overflow_y_scroll()
    };

    // Validate clears the run before it speaks, so a report is never drawn beside one.
    if let Some(report) = app.sink.script.report.as_ref() {
        let colour = if report.ok() {
            theme::success()
        } else {
            theme::danger()
        };
        return shell()
            .child(slab(colour).id("script-verdict").p_2().child(
                mono(report.verdict(), colour).text_size(theme::font(Family::Chrome, Role::Meta)),
            ))
            .children(
                report
                    .errors
                    .iter()
                    .enumerate()
                    .map(|(index, diagnostic)| diagnostic_row(index, diagnostic)),
            )
            .into_any_element();
    }

    let Some(outcome) = app.sink.script.outcome.as_ref() else {
        return shell()
            .child(
                mono(
                    "nothing run yet \u{2014} press Run, or Validate to check without running",
                    theme::text_faint(),
                )
                .text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .into_any_element();
    };

    // The returned value and the error are mutually exclusive in practice, but both are drawn if
    // both are there: a run that printed, returned and then threw has three things to say, and
    // hiding one of them would make the page lie about what happened.
    let result = outcome.value.as_ref().map(|value| {
        slab(theme::success())
            .id("script-result")
            .p_2()
            .mt_1()
            .child(
                mono(format!("\u{2190} {value}"), theme::success())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .into_any_element()
    });

    let error = outcome.error.as_ref().map(|reason| {
        slab(theme::danger())
            .id("script-error")
            .p_2()
            .mt_1()
            .child(
                mono(reason.clone(), theme::danger())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .into_any_element()
    });

    let footer = match outcome.ui.as_ref() {
        Some(ui) if ui.handler => format!(
            "{} ms \u{00b7} a panel was declared, with a handler",
            outcome.elapsed_ms
        ),
        Some(_) => format!(
            "{} ms \u{00b7} a panel was declared, without a handler",
            outcome.elapsed_ms
        ),
        None => format!("{} ms", outcome.elapsed_ms),
    };

    shell()
        .children(outcome.lines.iter().enumerate().map(|(index, line)| {
            div()
                .id(("script-line", index))
                .child(
                    mono(line.text.clone(), level_colour(line.level))
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                )
                .into_any_element()
        }))
        // Every call that would have changed something, and did not. Drawn in the warning colour
        // rather than the log's: an intent is not output, it is an account of what was refused.
        .children(outcome.intents.iter().enumerate().map(|(index, intent)| {
            div()
                .id(("script-intent", index))
                .child(
                    mono(intent.line(), theme::warning())
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                )
                .into_any_element()
        }))
        // A run that printed nothing and returned nothing still happened, and saying so is what
        // separates it from a run that never started.
        .children(
            (outcome.lines.is_empty()
                && outcome.intents.is_empty()
                && outcome.value.is_none()
                && outcome.error.is_none())
            .then(|| {
                mono(
                    "ran, printed nothing, returned nothing",
                    theme::text_faint(),
                )
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .into_any_element()
            }),
        )
        .children(result)
        .children(error)
        .child(
            mono(footer, theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Micro))
                .mt_2(),
        )
        .into_any_element()
}

/// One problem oxc found, named with the buffer it is in: the page has two, and a line number
/// without one would send the reader to the wrong place.
fn diagnostic_row(index: usize, diagnostic: &script::Diagnostic) -> AnyElement {
    slab(theme::danger())
        .id(("script-diagnostic", index))
        .p_2()
        .mt_1()
        .gap_0p5()
        .child(
            mono(
                format!(
                    "{}:{}:{} \u{2014} {}",
                    diagnostic.buffer.label(),
                    diagnostic.line,
                    diagnostic.column,
                    diagnostic.message
                ),
                theme::danger(),
            )
            .text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .children((!diagnostic.snippet.is_empty()).then(|| {
            mono(diagnostic.snippet.clone(), theme::text_muted())
                .text_size(theme::font(Family::Chrome, Role::Micro))
                .into_any_element()
        }))
        .into_any_element()
}

/// The switch, and whichever of the two things it chose.
fn lower(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let pane = app.sink.script.pane;
    let declared = app.sink.script.ui.is_some();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    mono(
                        match pane {
                            ScriptPane::Docs => "Available to a script",
                            ScriptPane::Panel => "The panel this script declared",
                        },
                        theme::text_faint(),
                    )
                    .text_size(theme::font(Family::Chrome, Role::Micro)),
                )
                .child(div().flex_1().min_w(px(0.)))
                .children(ScriptPane::all().iter().map(|which| {
                    choice_pill(
                        eid("script-pane", which.label()),
                        // A dot on Panel when there is one: the switch is the only place the page
                        // can say a run declared something without stealing the half to show it.
                        match (which, declared) {
                            (ScriptPane::Panel, true) => format!("{} \u{2022}", which.label()),
                            _ => which.label().to_string(),
                        },
                        pane == *which,
                        handler_click(cx, move |this, cx| this.set_sink_script_pane(*which, cx)),
                    )
                    .into_any_element()
                })),
        )
        .child(match pane {
            ScriptPane::Docs => reference(),
            ScriptPane::Panel => panel(app, cx),
        })
        .into_any_element()
}

/// The reference: one Markdown string, rendered.
///
/// A document rather than a table of pairs, because the surface has namespaces now and a namespace
/// needs a paragraph and an example to be worth having. The string lives beside the bindings it
/// describes, in `state::script`, so adding one is adding a row there and nothing here.
fn reference() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .child(
            TextView::markdown("script-api-doc", script::API_DOC)
                .text_size(theme::font(Family::Content, Role::Meta))
                .p_4()
                .scrollable(true)
                .selectable(true),
        )
        .into_any_element()
}

/// The surface the script declared through `ubiq.sink.ui`.
///
/// The whole of the A2UI page's preview half, opened on whatever the last run declared — its
/// inputs write into its own model, and a button reports or refuses exactly as one on the A2UI
/// page does, and then re-runs the script that declared it.
fn panel(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let live = &app.sink.script.a2ui;

    let Some(surface) = live.surface.as_ref() else {
        return div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .p_4()
            .gap_2()
            .children(live.error.as_ref().map(|reason| {
                slab(theme::danger())
                    .id("script-panel-error")
                    .p_2()
                    .child(
                        mono(reason.clone(), theme::danger())
                            .text_size(theme::font(Family::Chrome, Role::Meta)),
                    )
                    .into_any_element()
            }))
            .children(live.error.is_none().then(|| {
                mono(
                    "no panel \u{2014} call ubiq.sink.ui(payload, handler) and press Run",
                    theme::text_faint(),
                )
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .into_any_element()
            }))
            .into_any_element();
    };

    let view = cx.entity();
    let on_tab = {
        let view = view.clone();
        Rc::new(
            move |id: SharedString, index: usize, _window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    this.select_script_a2ui_tab(id.to_string(), index, cx)
                });
            },
        )
    };
    let on_modal = {
        let view = view.clone();
        Rc::new(
            move |id: SharedString, _window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    this.toggle_script_a2ui_modal(id.to_string(), cx)
                });
            },
        )
    };
    let on_action = {
        let view = view.clone();
        Rc::new(
            move |key: SharedString, _window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    this.fire_script_a2ui_action(key.to_string(), cx)
                });
            },
        )
    };
    let on_value = {
        let view = view.clone();
        Rc::new(
            move |path: SharedString,
                  value: serde_json::Value,
                  _window: &mut Window,
                  cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    this.set_script_a2ui_value(path.to_string(), value, cx)
                });
            },
        )
    };

    let blocked = live
        .parsed()
        .map(|parsed| !crate::state::a2ui::action::blocking_checks(&parsed).is_empty())
        .unwrap_or(false);

    let ctx = crate::ui::a2ui::Ctx {
        surface,
        model: &live.model,
        fields: &live.fields,
        invalid: &live.invalid,
        blocked,
        tabs: &live.tabs,
        open_modal: live.modal.as_deref(),
        on_tab,
        on_modal,
        on_action,
        on_value,
    };

    div()
        .id("script-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .p_4()
        .gap_2()
        .overflow_y_scroll()
        .children(live.error.as_ref().map(|reason| {
            slab(theme::danger())
                .p_2()
                .mb_2()
                .child(
                    mono(
                        format!("{reason} \u{2014} showing the last surface that parsed"),
                        theme::danger(),
                    )
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
                )
                .into_any_element()
        }))
        .child(crate::ui::a2ui::render(&ctx))
        // What the panel has sent, newest first: the envelope a press would have carried, and the
        // note when a press had no handler to replay into.
        .children(live.log.iter().take(3).enumerate().map(|(index, entry)| {
            slab(theme::border())
                .id(("script-panel-log", index))
                .p_2()
                .child(
                    mono(entry.clone(), theme::text_muted())
                        .text_size(theme::font(Family::Chrome, Role::Micro)),
                )
                .into_any_element()
        }))
        .into_any_element()
}

/// What a console line's level is worth, from the status group.
fn level_colour(level: ConsoleLevel) -> gpui::Rgba {
    match level {
        ConsoleLevel::Log => theme::text_muted(),
        ConsoleLevel::Info => theme::info(),
        ConsoleLevel::Warn => theme::warning(),
        ConsoleLevel::Error => theme::danger(),
    }
}

/// The one line that says what the pane under it is.
fn caption(text: impl Into<String>) -> AnyElement {
    div()
        .flex_none()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(theme::border())
        .child(
            mono(text.into(), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Micro)),
        )
        .into_any_element()
}

/// A buffer at the inset the sink's other source panes draw one at.
fn buffer(state: &gpui::Entity<EditorState>) -> AnyElement {
    Editor::new(state)
        .h(relative(1.))
        .p_0()
        .border_0()
        .into_any_element()
}

/// A click handler over the window, in the shape `kit`'s buttons take.
fn handler_click(
    cx: &Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static {
    let view = cx.entity();
    move |_, _, cx| {
        view.update(cx, |this, cx| f(this, cx));
    }
}

fn half(child: AnyElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(child)
}
