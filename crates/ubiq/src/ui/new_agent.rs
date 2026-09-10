//! The New agent modal: every question a start answers, in one place.
//!
//! The body is drawn from [`crate::state::new_agent::NewAgentForm`] and nothing else, which is
//! why the settings page's profile form can draw the same rows — a profile is a saved answer to
//! the same questions, and two forms asking them differently is how the two drift apart.
//!
//! **Every row is one line**: the label and its hint mark on the left, the control on the right.
//! The words behind each hint are on the mark's hover rather than under the row, because a note
//! per row is what made this form taller than the window — see [`crate::ui::kit::label_hint`].
//!
//! **Everything below the first row is drawn inert until a target is chosen**, faint and taking
//! no click, rather than hidden: a form whose shape jumps as it is filled in is a form the user
//! has to re-read after every answer.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, ElementId, Entity, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, StatefulInteractiveElement as _, Styled, Window, anchored,
    deferred, div, px,
};
use gpui_component::input::Textarea;
use ubiq_proto::mcp::McpInfo;

use crate::app::{AppState, DialogConfirm, SubmitSearch};
use crate::state::navigator::subsequence;
use crate::state::new_agent::{NewAgentForm, OpenList, Purpose, Target};
use crate::state::workbench::HarnessChoice;
use crate::theme;
use crate::ui::kit::menu::{MENU_ANCHOR_UP, MODAL_MENU_LAYER};
use crate::ui::kit::{
    Picker, PickerStyle, check_box, elided, field, ghost_button, hint_row, label_hint, modal,
    primary_button, prompt_modal, slab,
};
use crate::ui::{eid, handler, indexed};

/// How wide every control in the right-hand column is drawn. One width for all of them, so the
/// column of pickers reads as a column rather than as a ragged edge.
const CONTROL_WIDTH: f32 = 230.;

/// The modal. Its body is [`body`]; only the title and the footer are its own.
pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.new_agent.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let ready = form.target.is_some();
    // Only a bare harness is worth writing down: a start already pointed at a profile would be
    // saving that profile back over itself.
    let savable = matches!(form.target, Some(Target::Harness { .. }));

    let mut actions = div().flex().items_center().gap_2();
    if savable {
        actions = actions.child(ghost_button(
            "new-agent-save-profile",
            None,
            "Save profile",
            cx.listener(|this, _, window, cx| this.open_new_agent_naming(window, cx)),
        ));
    }
    let footer = footer_row(
        app,
        actions
            .child(
                primary_button(
                    "new-agent-start",
                    None,
                    "Start",
                    cx.listener(|this, _, _, cx| {
                        this.start_new_agent(cx);
                    }),
                )
                .when(!ready, |button| button.opacity(0.5)),
            )
            .into_any_element(),
        cx,
    );

    let modal = modal(
        "new-agent",
        theme::accent(),
        "New agent",
        body(app, window, cx),
        footer,
        handler(&view, |this, _, cx| {
            // A picker's list is painted above this modal but *outside* its subtree, so a click
            // inside the list — the filter field most of all — reads to the modal as a click
            // outside itself. While a list is down it is the layer the click belongs to: the list
            // dismisses itself against its own bounds, and the form under it stays up. One
            // gesture peels one layer, which is what Escape does here too.
            if this
                .new_agent_form()
                .is_some_and(|form| form.open.is_some())
            {
                return;
            }
            this.close_new_agent(cx)
        }),
        window,
    );

    // The name question is painted over the form rather than instead of it: what is being named
    // is on screen behind it, which is the whole reason the prompt is worth reading.
    let named = !app.profile_id_input.read(cx).value().trim().is_empty();
    confirmable(div().child(modal), cx)
        .when(form.naming, |this| {
            this.child(prompt_modal(
                "new-agent-name",
                "Save profile",
                Some(
                    "A saved setup answers all of this at once. Saving over an existing name \
                     replaces it.",
                ),
                "Name",
                &app.profile_id_input,
                "Save",
                named,
                handler(&view, |this, _, cx| this.save_new_agent_profile(cx)),
                handler(&view, |this, _, cx| this.close_new_agent_naming(cx)),
                window,
                cx,
            ))
        })
        .into_any_element()
}

/// Make Enter and ⌘⏎ mean "do the thing this form is for", wherever they are pressed inside it.
///
/// No new binding: `DialogConfirm` is already Enter at the window and `SubmitSearch` is already
/// ⌘⏎ at the window **and** inside a field, so both arrive here — and a handler on the form runs
/// before the window's own, which is what takes the keystroke rather than sharing it.
///
/// Enter is deliberately not bound inside a field, so the opening-prompt textarea keeps it as a
/// newline; ⌘⏎ is what confirms from in there. Both are the settings page's profile form as well
/// as this modal's, which is why this wraps rather than being written into either.
pub fn confirmable(element: gpui::Div, cx: &mut Context<AppState>) -> gpui::Div {
    element
        .on_action(cx.listener(|this, _: &DialogConfirm, window, cx| {
            this.confirm_new_agent_form(window, cx)
        }))
        .on_action(
            cx.listener(|this, _: &SubmitSearch, window, cx| {
                this.confirm_new_agent_form(window, cx)
            }),
        )
}

/// The action row every form of this shape carries: what the start may be given on the left, what
/// the form is for on the right. Buttons are actions and belong together, rather than trailing the
/// questions as two more rows of the body.
///
/// `Custom policies` is still drawn faint and takes no click — the predisposition for something
/// the host does not answer yet. `MCPs` beside it is live: it opens the checklist of servers Ubiq
/// can inject, which the host does answer.
pub fn footer_row(app: &AppState, actions: AnyElement, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    // Drawn, faint, and taking no click.
                    div().opacity(0.5).child(ghost_button(
                        "new-agent-policies",
                        None,
                        "Custom policies",
                        |_, _, _| {},
                    )),
                )
                .child(mcps_button(app, cx)),
        )
        .child(actions)
        .into_any_element()
}

/// The MCPs trigger, with the checklist hanging off it while it is down.
///
/// The count rides in the label the way the rest of the window summarises a multiple answer — a
/// control whose whole value is behind a click has to say, closed, that there is something behind
/// it. Nothing on offer means no click: an empty panel is worse than a dead button, because the
/// button at least does not claim the host answered.
fn mcps_button(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let form = app.new_agent_form();
    let chosen = form.map(|form| form.mcps.len()).unwrap_or_default();
    let open = form.is_some_and(|form| form.open == Some(OpenList::Mcps));
    let offered = !app.workbench.mcps.is_empty();
    let label = match chosen {
        0 => "MCPs".to_string(),
        n => format!("MCPs \u{00b7} {n}"),
    };

    // Until the host answers there is nothing to tick, so the button is drawn faint and wired to
    // nothing rather than opening a panel that would say "none" — the same "inert" the rows in
    // [`body`] use, and the same shape a first answer turns live without moving anything.
    let toggle = cx.listener(move |this, _, window, cx| {
        if offered {
            this.toggle_new_agent_list(OpenList::Mcps, window, cx);
        }
    });
    let button = ghost_button("new-agent-mcps", None, label, toggle)
        // The panel is pinned to this button's own bottom-left corner, so the button has to be the
        // box that corner is measured from — the same reason `Picker`'s trigger is relative.
        .relative();

    div()
        .when(!offered, |this| this.opacity(0.5))
        .child(match offered && open {
            true => button.child(mcp_panel(app, &view)),
            false => button,
        })
        .into_any_element()
}

/// The checklist itself: one row per server Ubiq can inject, ticked where this form asks for it.
///
/// **Not a [`Picker`].** A picker answers one question with one row, and this answers "which of
/// these", which is a set — so it is the `deferred` + `anchored` panel a picker is built on, with
/// check boxes in it instead of a column of ticks. It opens *upward*, because the button that
/// raises it sits on the form's footer and a list dropping from there would drop off the window.
///
/// Painted at [`MODAL_MENU_LAYER`]: both surfaces that draw this footer are modals, so the layer
/// that clears a modal is the layer either of them needs — the settings page's profile form is a
/// modal over the page, not a page of its own.
fn mcp_panel(app: &AppState, view: &Entity<AppState>) -> AnyElement {
    let chosen = app
        .new_agent_form()
        .map(|form| form.mcps.clone())
        .unwrap_or_default();
    let rows: Vec<AnyElement> = app
        .workbench
        .mcps
        .iter()
        .map(|server| mcp_row(server, chosen.iter().any(|it| it == &server.name), view))
        .collect();

    deferred(
        anchored()
            .anchor(MENU_ANCHOR_UP)
            .snap_to_window_with_margin(px(8.))
            .child(
                div()
                    .id("new-agent-mcps-panel")
                    .w(px(MCP_PANEL_WIDTH))
                    .max_h(px(MCP_PANEL_MAX_HEIGHT))
                    .p_1()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .bg(theme::surface_raised())
                    .border_l(px(theme::accent_edge()))
                    .border_color(theme::accent())
                    .shadow_lg()
                    .children(rows)
                    // Painted above the modal that raised it, so a click on a row sits at the same
                    // screen point as a control underneath — the rule `kit::menu`'s own panel
                    // states, for the same reason.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down_out({
                        let view = view.clone();
                        move |_, window, cx| {
                            view.update(cx, |this, cx| this.dismiss_new_agent_list(window, cx));
                        }
                    }),
            ),
    )
    .priority(MODAL_MENU_LAYER)
    .into_any_element()
}

/// One server: the tick, its title, what it is for, and what it answers.
///
/// Three lines rather than one, which is the exception the rest of the window's rows are not: a
/// server is picked on what its tools are, and a name alone — `test`, `project-info` — says
/// nothing about that. Each line is still one line, elided with the whole of itself on hover.
fn mcp_row(server: &McpInfo, checked: bool, view: &Entity<AppState>) -> AnyElement {
    let name = server.name.clone();
    let tools = server
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>()
        .join(" \u{00b7} ");

    let mut lines = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .gap_0p5()
        .child(elided(
            eid("new-agent-mcp-title", &server.name),
            server.title.clone(),
            theme::text(),
            theme::font(theme::Family::Chrome, theme::Role::Body),
        ))
        .child(elided(
            eid("new-agent-mcp-note", &server.name),
            server.description.clone(),
            theme::text_muted(),
            theme::font(theme::Family::Chrome, theme::Role::Micro),
        ));
    // A server with no tools says nothing rather than drawing an empty line under its own name.
    if !tools.is_empty() {
        lines = lines.child(elided(
            eid("new-agent-mcp-tools", &server.name),
            tools,
            theme::text_faint(),
            theme::font(theme::Family::Chrome, theme::Role::Micro),
        ));
    }

    div()
        .id(eid("new-agent-mcp", &server.name))
        .px_2()
        .py_1p5()
        .flex()
        .items_start()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        // **The row owns the click, not the box.** An 18px tick is a small target for a row three
        // lines tall, and wiring both would toggle twice and land back where it started.
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |this, cx| this.toggle_new_agent_mcp(name.clone(), cx));
            }
        })
        .child(div().pt_0p5().child(check_box(
            eid("new-agent-mcp-box", &server.name),
            checked,
            |_, _, _| {},
        )))
        .child(lines)
        .into_any_element()
}

/// How wide the MCP checklist is drawn. Wider than a dropdown, because its rows carry a sentence.
const MCP_PANEL_WIDTH: f32 = 340.;

/// How tall it grows before it scrolls.
const MCP_PANEL_MAX_HEIGHT: f32 = 320.;

/// The rows both forms share, top to bottom. Drawn from whichever form is up.
pub fn body(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.new_agent_form().cloned() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let harness = app
        .workbench
        .agent_types
        .iter()
        .find(|it| it.id == form.agent_type);
    // Every row below the first answers a question the target has already narrowed, so none of
    // them means anything until it is answered.
    let live = form.target.is_some();
    // A target that is a harness *is* the harness and the identity, both answered at once. The two
    // rows that ask them again stay only for a profile, where overriding what the profile named is
    // the point of having them.
    let from_profile = matches!(form.target, Some(Target::Profile(_)));

    let mut rows = div().flex().flex_col().gap_1().pt_1();

    // 1 ─ what is being started. The one row that is always live, and the one the rest of the
    // form is downstream of — so it is drawn as a question of its own rather than as the first of
    // a column: its label above it, the control the full width of the body, and a gap under it
    // that says everything below is a consequence of this answer.
    rows = rows.child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pb_2()
            .child(label_hint(
                "new-agent-target-hint",
                "Agent",
                "A harness signed into an account starts fresh; a profile starts with every \
                 answer below already given.",
            ))
            .child(picker_of(
                app,
                &view,
                &form,
                "new-agent-target",
                "Choose\u{2026}",
                target_rows(app, &form),
                form.target.clone(),
                OpenList::Target,
                true,
                window,
                cx,
                |this, target, window, cx| this.pick_new_agent_target(target, window, cx),
            )),
    );

    // 2 ─ the harness and the identity, as one control, because they are one question: the first
    // row offers them together and a profile is overridden the same way it was chosen. Drawn only
    // for a profile — a target that is a pair has already answered this.
    if from_profile {
        rows = rows.child(picker_row(
            app,
            &view,
            &form,
            "new-agent-harness",
            "Harness",
            "Which tool this runs, as whom.",
            "Choose\u{2026}",
            pair_rows(app),
            (!form.agent_type.is_empty()).then(|| (form.agent_type.clone(), form.account.clone())),
            OpenList::Harness,
            live,
            window,
            cx,
            |this, (agent_type, account), window, cx| {
                this.pick_new_agent_pair(agent_type, account, window, cx)
            },
        ));
    }

    // 4 ─ what it runs as: the model, how hard it thinks and what it may do without asking, in one
    // block. Three answers to one question, and a block is what says so.
    let mut engine = slab(theme::border()).px_2().py_1().gap_1();

    // A probe is slow exactly once, and while it runs the trigger says so rather than reading as a
    // harness with nothing on offer.
    let model_rows: Vec<(String, Option<String>)> = form
        .models
        .iter()
        .map(|it| (it.id.clone(), Some(it.id.clone())))
        .collect();
    engine = engine.child(picker_row(
        app,
        &view,
        &form,
        "new-agent-model",
        "Model",
        "What the harness runs on. Leave it to start on the harness's own.",
        if form.probing {
            "Asking the harness\u{2026}"
        } else {
            "The harness's own"
        },
        model_rows,
        form.model.clone(),
        OpenList::Model,
        live && !form.probing,
        window,
        cx,
        |this, model, window, cx| this.pick_new_agent_model(model, window, cx),
    ));

    // The reasoning level and the permission mode are **always drawn**, even where this model has
    // no levels or this harness advertises no modes. Both are questions of the harness, answered
    // by a probe that takes a moment, and a block that grew a row when the answer landed moved
    // everything under it after the user had already started reading. A row with nothing to offer
    // is inert and says what it would have offered instead.
    let levels = form.model_levels();
    let mut level_rows: Vec<(String, Option<Option<String>>)> =
        vec![("The model's own".to_string(), Some(None))];
    level_rows.extend(
        levels
            .iter()
            .map(|it| (it.name.clone(), Some(Some(it.value.clone())))),
    );
    engine = engine.child(picker_row(
        app,
        &view,
        &form,
        "new-agent-thinking",
        "Thinking effort",
        "How hard the model is asked to think. Not every model has the knob.",
        if form.probing {
            "Asking the harness\u{2026}"
        } else {
            "The model's own"
        },
        level_rows,
        Some(form.thinking.clone()),
        OpenList::Thinking,
        live && !form.probing && !levels.is_empty(),
        window,
        cx,
        |this, thinking, window, cx| this.pick_new_agent_thinking(thinking, window, cx),
    ));

    let modes = harness.map(|it| it.modes.as_slice()).unwrap_or_default();
    let mut mode_rows: Vec<(String, Option<Option<String>>)> =
        vec![("The harness's own".to_string(), Some(None))];
    mode_rows.extend(
        modes
            .iter()
            .map(|it| (it.name.clone(), Some(Some(it.value.clone())))),
    );
    engine = engine.child(picker_row(
        app,
        &view,
        &form,
        "new-agent-mode",
        "Mode",
        "What it may do without asking. A new agent starts on the harness's unattended mode.",
        "The harness's own",
        mode_rows,
        Some(form.mode.clone()),
        OpenList::Mode,
        live && !modes.is_empty(),
        window,
        cx,
        |this, mode, window, cx| this.pick_new_agent_mode(mode, window, cx),
    ));
    rows = rows.child(engine);

    // 5 ─ the one control here with nothing behind it. Drawn, faint, and taking no click.
    rows = rows.child(
        div().opacity(0.5).child(hint_row(
            "new-agent-persistent-hint",
            "Persistent",
            "Persists across Ubiq restarts \u{2014} not yet built.",
            div()
                .flex()
                .flex_none()
                .justify_end()
                .w(px(CONTROL_WIDTH))
                .child(check_box("new-agent-persistent", false, |_, _, _| {}))
                .into_any_element(),
        )),
    );

    // 6 ─ the subagent ceiling. Never a launch flag: it is written into the preamble the first
    // turn carries, which is the only place a harness would read it.
    let mut subagent_rows: Vec<(String, Option<Option<u8>>)> =
        vec![("Say nothing about it".to_string(), Some(None))];
    subagent_rows.extend((0..=10u8).map(|n| (n.to_string(), Some(Some(n)))));
    rows = rows.child(picker_row(
        app,
        &view,
        &form,
        "new-agent-subagents",
        "Max subagents",
        "Said to the agent as a directive \u{2014} no harness has a flag for it.",
        "Say nothing about it",
        subagent_rows,
        Some(form.max_subagents),
        OpenList::Subagents,
        live,
        window,
        cx,
        |this, max, window, cx| this.pick_new_agent_subagents(max, window, cx),
    ));

    // 7 ─ the opening words. The one control too tall to sit beside its label, so its label sits
    // above it.
    let prompt_focused = app
        .new_agent_prompt
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    rows = rows.child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_1()
            .when(!live, |this| this.opacity(0.5))
            .child(label_hint(
                "new-agent-prompt-hint",
                "Initial prompt",
                "Said to the agent first, under the subagent directive. Neither is shown in the \
                 transcript.",
            ))
            .child(
                field(theme::border(), prompt_focused)
                    .flex_col()
                    .items_stretch()
                    .child(
                        div().px_2().py_1p5().cursor_text().child(
                            Textarea::new(&app.new_agent_prompt)
                                .appearance(false)
                                .bordered(false)
                                .w_full()
                                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body)),
                        ),
                    ),
            ),
    );

    rows.into_any_element()
}

/// The first row's list: the harness-and-identity pairs this machine has signed in and — for a
/// start — the setups already written down, under their own headings with a hairline between.
///
/// [`crate::state::WorkbenchState::harness_choices`] is what groups them, so this list and every
/// other harness list in the window read the same rows in the same order. A pair whose harness is
/// not installed here is drawn disabled rather than dropped: "not installed" is worth saying, and
/// installing it is the fix.
fn target_rows(app: &AppState, form: &NewAgentForm) -> Vec<(String, Option<Target>)> {
    let profiles = &app.workbench.settings.profiles;
    // A profile is a saved answer to this form, so a profile form does not offer one as its own
    // starting point.
    let offered: &[ubiq_proto::messages::ProfileInfo] = match form.purpose {
        Purpose::Start => profiles,
        Purpose::Profile => &[],
    };
    app.workbench
        .harness_choices(&app.workbench.settings.accounts, offered)
        .into_iter()
        .filter_map(|choice| match choice {
            HarnessChoice::Label(label) => Some((label.to_string(), None)),
            HarnessChoice::Separator => Some((String::new(), None)),
            HarnessChoice::Pair { harness, account } => {
                let harness = app.workbench.agent_types.get(harness)?;
                let target = harness.available.then(|| Target::Harness {
                    agent_type: harness.id.clone(),
                    account: Some(account.clone()),
                });
                Some((format!("{} \u{00b7} {account}", harness.label), target))
            }
            HarnessChoice::Profile(index) => offered
                .get(index)
                .map(|it| (it.id.clone(), Some(Target::Profile(it.id.clone())))),
            // Never offered: a bare harness with no identity is what this form is for asking about,
            // not something to start.
            HarnessChoice::Harness(_) => None,
        })
        .collect()
}

/// A harness and the identity it runs as — the pair the first row offers, and the one thing the
/// harness override picks. Named because it rides in a row list, and a row list of them is a type
/// nobody should have to read twice.
type Pair = (String, Option<String>);

/// One labelled row holding one dropdown.
///
/// A row with `enabled: false` is drawn faint and wired to nothing, which is what "inert" means
/// here — the list cannot open, so it cannot be picked from either. The caller narrows the rows
/// itself, because [`Picker`] does not filter, and the values ride alongside the labels so a pick
/// resolves by position in the list that was actually drawn.
#[allow(clippy::too_many_arguments)]
fn picker_row<T: Clone + PartialEq + 'static>(
    app: &AppState,
    view: &Entity<AppState>,
    form: &NewAgentForm,
    id: &'static str,
    label: &str,
    note: &str,
    placeholder: &str,
    rows: Vec<(String, Option<T>)>,
    chosen: Option<T>,
    list: OpenList,
    enabled: bool,
    window: &Window,
    cx: &App,
    pick: impl Fn(&mut AppState, T, &mut Window, &mut Context<AppState>) + 'static,
) -> AnyElement {
    let picker = picker_of(
        app,
        view,
        form,
        id,
        placeholder,
        rows,
        chosen,
        list,
        enabled,
        window,
        cx,
        pick,
    );
    div()
        .when(!enabled, |this| this.opacity(0.5))
        .child(hint_row(
            ElementId::Name(format!("{id}-hint").into()),
            label,
            note,
            div()
                .flex_none()
                .w(px(CONTROL_WIDTH))
                .child(picker)
                .into_any_element(),
        ))
        .into_any_element()
}

/// The dropdown itself, without the row around it — what the lead question draws at the body's
/// full width and what [`picker_row`] draws in the right-hand column.
#[allow(clippy::too_many_arguments)]
fn picker_of<T: Clone + PartialEq + 'static>(
    app: &AppState,
    view: &Entity<AppState>,
    form: &NewAgentForm,
    id: &'static str,
    placeholder: &str,
    rows: Vec<(String, Option<T>)>,
    chosen: Option<T>,
    list: OpenList,
    enabled: bool,
    window: &Window,
    cx: &App,
    pick: impl Fn(&mut AppState, T, &mut Window, &mut Context<AppState>) + 'static,
) -> Picker {
    let open = enabled && form.open == Some(list);
    let trigger = rows
        .iter()
        .find(|(_, value)| value.is_some() && *value == chosen)
        .map(|(label, _)| label.clone())
        .unwrap_or_else(|| placeholder.to_string());

    // A filter drops the headings with the rows they head: a group line above nothing reads as a
    // group with nothing in it.
    let needle = app.picker_search.read(cx).value().trim().to_lowercase();
    let shown: Vec<(String, Option<T>)> = if open && !needle.is_empty() {
        rows.into_iter()
            .filter(|(label, value)| value.is_some() && subsequence(&needle, label))
            .collect()
    } else {
        rows
    };

    let disabled: Vec<usize> = shown
        .iter()
        .enumerate()
        .filter(|(_, (_, value))| value.is_none())
        .map(|(ix, _)| ix)
        .collect();
    // A heading with no words is the hairline between two groups.
    let separators: Vec<usize> = shown
        .iter()
        .enumerate()
        .filter(|(_, (label, value))| value.is_none() && label.is_empty())
        .map(|(ix, _)| ix)
        .collect();
    let selected = shown
        .iter()
        .position(|(_, value)| value.is_some() && *value == chosen);
    let values: Vec<Option<T>> = shown.iter().map(|(_, value)| value.clone()).collect();
    let labels: Vec<String> = shown.into_iter().map(|(label, _)| label).collect();

    let mut picker = Picker::new(ElementId::Name(id.into()), trigger)
        .items(labels)
        .disabled(disabled)
        .separators(separators)
        .open(open)
        .style(PickerStyle::Field)
        .above_modal();
    if let Some(selected) = selected {
        picker = picker.selected(selected);
    }
    if enabled {
        let focused = app
            .picker_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        picker = picker
            .search(&app.picker_search, focused)
            .on_toggle(handler(view, move |this, window, cx| {
                this.toggle_new_agent_list(list, window, cx)
            }))
            .on_pick(indexed(view, move |this, index, window, cx| {
                if let Some(Some(value)) = values.get(index).cloned() {
                    pick(this, value, window, cx);
                }
            }))
            .on_dismiss(handler(view, |this, window, cx| {
                this.dismiss_new_agent_list(window, cx)
            }));
    }
    picker
}

/// Every harness-and-identity this machine has signed in, labelled as the first row labels them.
///
/// The same pairs, read the same way, so overriding a profile's harness is the same gesture as
/// choosing one — and a harness with no identity is no more startable here than it is there.
fn pair_rows(app: &AppState) -> Vec<(String, Option<Pair>)> {
    app.workbench
        .harness_choices(&app.workbench.settings.accounts, &[])
        .into_iter()
        .filter_map(|choice| match choice {
            HarnessChoice::Label(label) => Some((label.to_string(), None)),
            HarnessChoice::Separator => Some((String::new(), None)),
            HarnessChoice::Pair { harness, account } => {
                let harness = app.workbench.agent_types.get(harness)?;
                let value = harness
                    .available
                    .then(|| (harness.id.clone(), Some(account.clone())));
                Some((format!("{} \u{00b7} {account}", harness.label), value))
            }
            HarnessChoice::Harness(_) | HarnessChoice::Profile(_) => None,
        })
        .collect()
}
