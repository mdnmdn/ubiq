//! Task sync on screen: **one declarative renderer**, and the four surfaces around it.
//!
//! This module is the single largest thing the task-source layer buys. Every provider — Trello in
//! the base, Azure DevOps and ClickUp in Studio — is configured through the code below, and
//! **none of it names one**. A provider declares `&'static [ConfigField]`; [`config_field`] draws
//! it; a Test runs it. There is no per-provider branch anywhere in this file, and if one ever
//! became necessary that would be a hole in `R6` to report rather than an `if` to write.
//!
//! Four things are drawn here:
//!
//! | Surface | What it answers |
//! |---|---|
//! | [`settings_section`] | Which board, under what filter, mapped how, how often |
//! | [`import_dialog`] | Which of the offered items become tasks (`R12`) |
//! | [`sync_badge`] | What happened to **this** card — and `Parked` above all (`R9`) |
//! | [`status_item`] | Whether the board as a whole is in step |
//!
//! **How capabilities grey things out without a capability branch.** The plan's gate is exact: one
//! rendered section covers every provider *without a capability match in the drawing code*. The
//! mechanism is [`crate::state::tasksrc::ABILITIES`] and
//! [`crate::state::tasksrc::CAVEATS`] — tables whose rows carry an `fn(&ProviderCaps) -> bool`.
//! The draw path iterates them and asks `(row.needs)(&caps)`; it never names a flag. Adding a
//! capability is a row, not an arm.
//!
//! **Where the two surfaces meet (`X14`).** A second edition may draw its own gated pickers — a
//! connection with a Check, a collection, a project — **immediately above** this section, on the
//! same page. That is a layout obligation: drawn apart, a person cannot tell why picking a project
//! did not change what syncs. This section draws the *how*; the edition above it draws the
//! *where*.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName};

use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::tasksrc::{
    Authority, ConfigField, ConfigFieldKind, Direction, DriftSide, FieldDrift, LinkState,
    POLL_FLOOR, ProviderCaps, SyncState,
};
use ubiq_proto::work::{Kind, Priority, Status};

use crate::app::AppState;
use crate::ext::settings::{
    SectionCtx, SectionGate, SettingsContainer, SettingsSectionSpec, register,
};
use crate::ext::{Registry, ids};
use crate::state::tasksrc::{ABILITIES, CAVEATS, TaskSrcMenu};
use crate::state::{Layer, MenuId};
use crate::theme::{self, Family, Role};
use crate::ui::board::status_colour;
use crate::ui::kit::{
    Picker, PickerStyle, UbiqIcon, check_box, column, field, ghost_button, heading, mono,
    primary_button, setting_row, state_chip, status_dot, toggle_pill,
};
use crate::ui::{dismiss, handler, indexed};

/// What the label of a picker reads when nothing is picked. One string, so an unfilled `Choice`
/// and an unfilled lane row read the same way.
const UNSET: &str = "\u{2014}";

/// Adapt a method on the root view into the click handler the kit's buttons expect.
///
/// [`crate::ui::handler`] answers the kit's *menu* shape, `Fn(&mut Window, &mut App)`; a button's
/// is `Fn(&ClickEvent, …)`. Both are needed in this module — pickers take the first, buttons the
/// second — so both adapters are named rather than one being inlined at thirty call sites.
fn click(
    view: &Entity<AppState>,
    f: impl Fn(&mut AppState, &mut Window, &mut Context<AppState>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let view = view.clone();
    move |_, window, cx| {
        view.update(cx, |this, cx| f(this, window, cx));
    }
}

// ── The settings section ────────────────────────────────────────────

/// Register the task-sync section into the project dialog.
///
/// A genuine contribution, `ui::sink::ext_demo`'s shape exactly: it goes in through
/// [`crate::ext::settings::register`] and **nothing in `ext/settings.rs`, `ui/settings.rs` or
/// `ui/sink/project.rs` was touched to add it** — which is the only evidence that M2's container
/// is a container rather than a refactor.
pub fn settings_section(reg: &mut Registry<SettingsSectionSpec>) {
    register(
        reg,
        SettingsSectionSpec {
            id: ids::PROJECT_TASK_SYNC,
            container: SettingsContainer::Project,
            // Beside Tasks, in the group about what the project *is*: a board bound to a tracker
            // is the same question as the board, asked one level out.
            group: ids::SETTINGS_PROJECT_CORE,
            label: "Task sync",
            icon: || Icon::new(IconName::Globe),
            // A folder not yet in the catalogue has no board to bind, on Tools' and Tasks' own
            // footing.
            gate: SectionGate::WithRecord,
            count: Some(|ctx, _| {
                let n = ctx.app.tasksrc.links.len();
                (n > 0).then_some(n)
            }),
            // The page is the only thing that needs the provider list, so it is asked for on
            // arrival rather than at boot — a build that never opens this page never asks.
            on_show: Some(|app, cx| app.ask_task_providers(cx)),
            render: render_section,
        },
    );
}

fn render_section(
    ctx: &SectionCtx<'_>,
    window: &Window,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let app = ctx.app;
    let src = &app.tasksrc;
    let caps = src.caps();
    let view = cx.entity();

    let mut rows: Vec<AnyElement> = vec![heading(
        "Task sync",
        "Bind this project's board to a board somewhere else. Everything below the provider is \
         drawn from what that provider declared it needs — no part of this page knows which \
         tracker it is configuring.",
    )];

    // ── Which provider, and which identity ──────────────────────────
    rows.push(provider_row(app, &view));

    let Some(draft) = src.draft.as_ref() else {
        rows.push(note(
            "Nothing is bound. Pick a provider above and this page fills in with what it asked \
             for.",
        ));
        return column(rows).into_any_element();
    };

    // ── Which identity ──────────────────────────────────────────────
    rows.push(connection_row(app, &view));

    // ── The board ───────────────────────────────────────────────────
    let container_labels: Vec<SharedString> = src
        .containers
        .iter()
        .map(|c| SharedString::from(c.name.clone()))
        .collect();
    let bound = src.containers.iter().position(|c| c.id == draft.container);
    rows.push(setting_row(
        "Board",
        "What this project's tasks are bound to. Every other id on this page is only meaningful \
         inside it.",
        picker(
            "tasksrc-container",
            container_labels,
            bound,
            src.menu.as_ref() == Some(&TaskSrcMenu::Container),
            &view,
            TaskSrcMenu::Container,
            |this, index, cx| this.pick_task_container(index, cx),
        ),
    ));

    // ── The filter: the provider's declared schema, rendered ────────
    //
    // The one loop this whole milestone exists for. It reads the schema and nothing else.
    let Some(info) = src.provider() else {
        rows.push(note(
            "This binding names a provider this build does not have, so its fields cannot be \
             drawn. Nothing is lost — the binding is left exactly as it is.",
        ));
        return column(rows).into_any_element();
    };

    rows.push(sub_heading(
        "Filter",
        "Declared by the provider, drawn here. Test runs it.",
    ));
    for caveat in CAVEATS
        .iter()
        .filter(|c| c.at == "filter" && (c.when)(&caps))
    {
        rows.push(note(caveat.note));
    }
    for spec in &info.config_schema {
        rows.push(config_field(app, spec, window, &view, cx));
    }
    rows.push(test_row(app, &view));

    // ── The lane map ────────────────────────────────────────────────
    rows.push(sub_heading(
        "Lanes",
        "Which column a remote lane lands in. A lane left unset parks its tasks where they are \
         rather than moving them somewhere nobody chose.",
    ));
    let statuses = Status::all();
    for lane in &src.lanes {
        let key = lane.id.as_str().to_string();
        let mapped = draft.lanes.get(&key).copied();
        let mut labels: Vec<SharedString> = vec![SharedString::from(UNSET)];
        labels.extend(statuses.iter().map(|s| SharedString::from(s.label())));
        let selected = mapped
            .and_then(|status| statuses.iter().position(|s| *s == status))
            .map(|ix| ix + 1)
            .or(Some(0));
        let lane_key = key.clone();
        rows.push(map_row(
            lane.name.clone(),
            mapped.map(status_colour),
            picker(
                ElementId::Name(format!("tasksrc-lane-{key}").into()),
                labels,
                selected,
                src.menu.as_ref() == Some(&TaskSrcMenu::Lane(key.clone())),
                &view,
                TaskSrcMenu::Lane(key),
                move |this, index, cx| {
                    let status = index.checked_sub(1).map(|ix| statuses[ix]);
                    this.map_task_lane(lane_key.clone(), status, cx);
                },
            ),
        ));
    }
    if src.lanes.is_empty() {
        rows.push(note(
            "No lanes have been fetched yet. They arrive with the board's facets.",
        ));
    }

    // ── The kind and priority maps ──────────────────────────────────
    rows.push(sub_heading(
        "Kinds and priorities",
        "A word the provider used, mapped onto one of Ubiq's. Unmapped is left alone rather than \
         guessed — a tracker without a type field says it in a label, and the same map serves \
         both.",
    ));
    rows.push(word_map(app, &view, false));
    rows.push(word_map(app, &view, true));

    // ── Direction, authority, interval ──────────────────────────────
    rows.push(sub_heading(
        "How it runs",
        "The same five questions for every provider.",
    ));
    rows.push(direction_row(app, &view, caps));
    rows.push(authority_row(app, &view, caps));
    rows.push(interval_row(app, &view, caps));
    rows.push(auto_import_row(app, &view));

    // ── What this provider can and cannot do ────────────────────────
    //
    // Iterated, never matched. Every row is drawn whatever the answer, so two providers' pages are
    // the same shape and the difference between them is legible.
    rows.push(sub_heading(
        "What this provider accepts",
        "Lit where the provider accepts the write and the direction allows it. A row that is not \
         lit is not broken — it is a thing this tracker does not do.",
    ));
    let writing = draft.direction == Direction::TwoWay;
    rows.push(
        div()
            .w(relative(1.))
            .flex()
            .flex_wrap()
            .gap_2()
            .py_2()
            .children(ABILITIES.iter().map(|ability| {
                let able = (ability.needs)(&caps);
                let tooltip = SharedString::from(if able {
                    ability.note.to_string()
                } else {
                    ability.absent.to_string()
                });
                div()
                    .id(ElementId::Name(
                        format!("tasksrc-cap-{}", ability.key).into(),
                    ))
                    .child(state_chip(
                        ability.label,
                        if able && writing {
                            theme::success()
                        } else {
                            theme::text_faint()
                        },
                        1.0,
                    ))
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                    })
            }))
            .into_any_element(),
    );

    // ── What differs right now ──────────────────────────────────────
    rows.extend(drift_overview(app, &view));

    // ── Save, unbind, import ────────────────────────────────────────
    rows.push(actions_row(app, &view));

    column(rows).into_any_element()
}

// ── The drift overview ──────────────────────────────────────────────

/// One row per task that differs, naming the field, the two values and which way the switch would
/// settle it — with **Push** / **Pull** per row and **Push all** / **Pull all** over the lot.
///
/// **Placed here, immediately under the authority switch**, which the proposal left open as "a
/// modal, a dock panel, or a section of the detail panel" and `D188` answers. The switch is what
/// decides these rows, so the rows belong where a person can read the switch's effect and change
/// the switch in the same glance. A modal would have separated the rule from its consequences, and
/// the detail panel can only ever show one card's.
///
/// Draws nothing at all when nothing differs, which is the ordinary case.
fn drift_overview(app: &AppState, view: &Entity<AppState>) -> Vec<AnyElement> {
    let rows: Vec<(TaskId, &FieldDrift)> = app
        .tasksrc
        .links
        .values()
        .flat_map(|link| link.drift.iter().map(move |drift| (link.task, drift)))
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }

    let mut out = vec![sub_heading(
        "What differs",
        "Both sides of each field, and which way this binding's switch settles it. A row that \
         settles nowhere is one the provider will not take a write for, or one the remote mints \
         — no switch setting closes it.",
    )];
    out.push(
        div()
            .w(relative(1.))
            .flex()
            .items_center()
            .gap_2()
            .py_1()
            .child(ghost_button(
                "tasksrc-drift-push-all",
                None,
                "Push all",
                click(view, |this, _, cx| {
                    this.resolve_all_drift(DriftSide::Push, cx)
                }),
            ))
            .child(ghost_button(
                "tasksrc-drift-pull-all",
                None,
                "Pull all",
                click(view, |this, _, cx| {
                    this.resolve_all_drift(DriftSide::Pull, cx)
                }),
            ))
            .into_any_element(),
    );
    for (task, drift) in rows {
        out.push(drift_row(view, task, drift));
    }
    out
}

/// One differing field: its name, the two values, the note, and the two buttons.
fn drift_row(view: &Entity<AppState>, task: TaskId, drift: &FieldDrift) -> AnyElement {
    let settles = match drift.settles {
        Some(side) => (side.label(), theme::info()),
        None => ("Stuck", theme::warning()),
    };
    let field = drift.field.clone();
    div()
        .id(ElementId::Name(
            format!("tasksrc-drift-{task}-{field}").into(),
        ))
        .w(relative(1.))
        .flex()
        .flex_col()
        .gap_1()
        .py_1p5()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(mono(drift.field.clone(), theme::text_strong()))
                .child(state_chip(settles.0, settles.1, 1.0))
                .child(ghost_button(
                    ElementId::Name(format!("tasksrc-drift-push-{task}-{field}").into()),
                    None,
                    "Push",
                    click(view, move |this, _, cx| {
                        this.resolve_task_drift(task, DriftSide::Push, cx)
                    }),
                ))
                .child(ghost_button(
                    ElementId::Name(format!("tasksrc-drift-pull-{task}-{field}").into()),
                    None,
                    "Pull",
                    click(view, move |this, _, cx| {
                        this.resolve_task_drift(task, DriftSide::Pull, cx)
                    }),
                )),
        )
        .child(drift_value("here", &drift.local))
        .child(drift_value("remote", &drift.remote))
        .child(note(&drift.note))
        .into_any_element()
}

/// One side's value, labelled. Truncated in the layout rather than in the string: what is shown is
/// evidence, and a value cut short in the middle of the difference proves nothing.
fn drift_value(side: &str, value: &str) -> AnyElement {
    div()
        .w(relative(1.))
        .flex()
        .gap_2()
        .child(
            div()
                .flex_none()
                .w(px(56.))
                .child(mono(side.to_string(), theme::text_faint())),
        )
        .child(
            div()
                .max_w(px(500.))
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(SharedString::from(if value.is_empty() {
                    UNSET.to_string()
                } else {
                    value.to_string()
                })),
        )
        .into_any_element()
}

/// **The renderer.** One `ConfigField`, drawn.
///
/// The only `match` in it is over [`ConfigFieldKind`] — the five shapes `R6` declares — and that
/// is the point rather than a compromise: the schema is a closed set the contract crate owns, and
/// a provider that needs a sixth shape is a provider that cannot ship until `ConfigFieldKind`
/// grows one. That cost is visible as a missing variant, which is exactly what `R6` promised
/// instead of a leak.
///
/// A `Secret` never draws a value. The interface does not have one: a secret's material went to
/// the connector family's secret store when it was saved, and the binding holds a reference.
/// Drawing a masked row of dots for a value nobody here holds would be a lie about what is stored.
fn config_field(
    app: &AppState,
    spec: &ConfigField,
    _window: &Window,
    view: &Entity<AppState>,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let src = &app.tasksrc;
    let Some(draft) = src.draft.as_ref() else {
        return div().into_any_element();
    };
    let key = spec.key.to_string();
    let label = if spec.required {
        format!("{} *", spec.label)
    } else {
        spec.label.to_string()
    };

    let control: AnyElement = match spec.kind {
        ConfigFieldKind::Text => text_control(app, &key, false),
        ConfigFieldKind::Secret => text_control(app, &key, true),
        ConfigFieldKind::Bool => {
            let on = draft.filter.flag(&key);
            let flag_key = key.clone();
            check_box(
                ElementId::Name(format!("tasksrc-bool-{key}").into()),
                on,
                click(view, move |this, _, cx| {
                    this.set_task_filter_flag(flag_key.clone(), !on, cx)
                }),
            )
            .into_any_element()
        }
        ConfigFieldKind::Choice(facet) => {
            let values = src.facets.get(facet);
            let mut labels: Vec<SharedString> = vec![SharedString::from(UNSET)];
            labels.extend(values.iter().map(|v| SharedString::from(v.label.clone())));
            let current = draft.filter.one(&key);
            let selected = current
                .and_then(|id| values.iter().position(|v| v.id == id))
                .map(|ix| ix + 1)
                .or(Some(0));
            let ids: Vec<String> = values.iter().map(|v| v.id.clone()).collect();
            let pick_key = key.clone();
            picker(
                ElementId::Name(format!("tasksrc-choice-{key}").into()),
                labels,
                selected,
                src.menu.as_ref() == Some(&TaskSrcMenu::Field(key.clone())),
                view,
                TaskSrcMenu::Field(key.clone()),
                move |this, index, cx| {
                    let chosen = index.checked_sub(1).and_then(|ix| ids.get(ix).cloned());
                    this.set_task_filter(pick_key.clone(), chosen.into_iter().collect(), cx);
                },
            )
        }
        ConfigFieldKind::MultiChoice(facet) => {
            let values = src.facets.get(facet);
            let chosen = draft.filter.many(&key);
            let pick_key = key.clone();
            let ids: Vec<String> = values.iter().map(|v| v.id.clone()).collect();
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap_1p5()
                .children(values.iter().enumerate().map(|(ix, value)| {
                    let on = chosen.iter().any(|held| held == &value.id);
                    let key_for_click = pick_key.clone();
                    let id = ids[ix].clone();
                    toggle_pill(
                        ElementId::Name(format!("tasksrc-multi-{pick_key}-{ix}").into()),
                        value.label.clone(),
                        theme::accent(),
                        on,
                        click(view, move |this, _, cx| {
                            this.toggle_task_filter_value(key_for_click.clone(), id.clone(), cx)
                        }),
                    )
                }))
                .when(values.is_empty(), |this| {
                    this.child(mono("nothing to pick from", theme::text_faint()))
                })
                .into_any_element()
        }
    };

    let _ = cx;
    setting_row(&label, field_note(spec), control)
}

/// What a field's row says under its label.
///
/// Built from the **declared** facts — required, testable, and the kind — and never from the
/// provider's name. `MultiChoice` gets the sentence that matters most and is least guessable: an
/// empty set is no constraint, not "match nothing".
fn field_note(spec: &ConfigField) -> &'static str {
    match spec.kind {
        ConfigFieldKind::Text if spec.testable => {
            "Free text, passed to the provider exactly as typed. Test runs it; nothing here parses \
             it."
        }
        ConfigFieldKind::Text => "Free text, passed to the provider exactly as typed.",
        ConfigFieldKind::Secret => {
            "Filed in the secret store when this page is saved. It is never read back here, which \
             is why the box is empty rather than showing dots for a value the interface does not \
             hold."
        }
        ConfigFieldKind::Bool => "On or off.",
        ConfigFieldKind::Choice(_) => "One of what the board offers.",
        ConfigFieldKind::MultiChoice(_) => {
            "Any of what the board offers. Picking none is no constraint, not \u{201c}match \
             nothing\u{201d}."
        }
    }
}

/// A `Text` or `Secret` box, over the buffer `ensure_tasksrc_inputs` keeps for this key.
///
/// The field is plain either way: this kit has no masked input, so a pasted token is on screen
/// until the page closes — the same bargain, and the same words, the connector family's own token
/// step already takes. What makes a `Secret` different is not the drawing, it is where the value
/// goes: to the secret store on save, never into the binding file, and never read back here.
// ponytail: unmasked secret field. Masking belongs in `kit::field`, not here.
fn text_control(app: &AppState, key: &str, secret: bool) -> AnyElement {
    let _ = secret;
    let Some(input) = app.tasksrc.inputs.get(key) else {
        // The buffer arrives on the frame after the schema does. A box that is not there yet is
        // drawn as absent rather than as an error.
        return mono("\u{2026}", theme::text_faint()).into_any_element();
    };
    field(theme::border(), false)
        .w(px(280.))
        .child(Input::new(input).appearance(false))
        .into_any_element()
}

/// **Test**: run the filter, and say how many came back.
///
/// `R7`, literally. It is **not** a syntax check — for Trello there is no syntax, and for a
/// tracker with a query language its own server is the only thing that knows the grammar. What it
/// proves is stronger than a parse: the count and the first few titles are what the filter
/// *returns*, so a query that parses and matches nothing is visibly wrong here rather than
/// silently wrong on the next pass.
fn test_row(app: &AppState, view: &Entity<AppState>) -> AnyElement {
    let src = &app.tasksrc;
    let testable = src
        .provider()
        .is_some_and(|info| info.config_schema.iter().any(|f| f.testable))
        || src.draft.is_some();
    if !testable {
        return div().into_any_element();
    }

    let running = src.testing.is_some();
    let result: AnyElement = match (running, src.test.as_ref()) {
        (true, _) => mono("running\u{2026}", theme::text_muted()).into_any_element(),
        (false, Some(test)) => div()
            .flex()
            .flex_col()
            .gap_1()
            .items_end()
            .child(mono(
                format!(
                    "{} item{} matched",
                    test.count,
                    if test.count == 1 { "" } else { "s" }
                ),
                if test.count == 0 {
                    theme::warning()
                } else {
                    theme::success()
                },
            ))
            .children(test.sample.iter().take(3).map(|title| {
                div()
                    .max_w(px(320.))
                    .truncate()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(title.clone()))
            }))
            .into_any_element(),
        (false, None) => mono("not run", theme::text_faint()).into_any_element(),
    };

    setting_row(
        "Test",
        "Runs the filter against the board, read-only, and reports what came back. It checks no \
         syntax: where a provider has a query language, that provider's own server is what \
         validates it.",
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(result)
            .child(primary_button(
                "tasksrc-test",
                Some(IconName::Play),
                "Test",
                click(view, |this, _, cx| this.test_task_source(cx)),
            ))
            .into_any_element(),
    )
}

/// One half of the kind/priority map: the entries already made, plus a picker over the candidate
/// words the provider supplied.
fn word_map(app: &AppState, view: &Entity<AppState>, priority: bool) -> AnyElement {
    let src = &app.tasksrc;
    let Some(draft) = src.draft.as_ref() else {
        return div().into_any_element();
    };
    let candidates = src.candidates();
    let label = if priority { "Priorities" } else { "Kinds" };

    let mut children: Vec<AnyElement> = Vec::new();
    let entries: Vec<(String, String)> = if priority {
        draft
            .priorities
            .iter()
            .map(|(word, value)| (word.clone(), value.label().unwrap_or("normal").to_string()))
            .collect()
    } else {
        draft
            .kinds
            .iter()
            .map(|(word, value)| (word.clone(), value.label().to_string()))
            .collect()
    };

    for (word, mapped) in &entries {
        let removing = word.clone();
        children.push(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(mono(word.clone(), theme::text_muted()))
                .child(mono("\u{2192}", theme::text_faint()))
                .child(mono(mapped.clone(), theme::text()))
                .child(ghost_button(
                    ElementId::Name(format!("tasksrc-unmap-{priority}-{word}").into()),
                    Some(IconName::Close),
                    "",
                    click(view, move |this, _, cx| {
                        this.map_task_word(removing.clone(), None, priority, cx)
                    }),
                ))
                .into_any_element(),
        );
    }

    // The add control: a word from the provider's own facets, then one of Ubiq's values. Two
    // pickers rather than a grid, because the candidate list is as long as the board's vocabulary.
    let words: Vec<SharedString> = candidates
        .iter()
        .map(|word| SharedString::from(word.to_string()))
        .collect();
    let owned: Vec<String> = candidates.iter().map(|word| word.to_string()).collect();
    let menu = if priority {
        TaskSrcMenu::Priority(String::new())
    } else {
        TaskSrcMenu::Kind(String::new())
    };
    let open = src.menu.as_ref() == Some(&menu);
    children.push(picker(
        ElementId::Name(format!("tasksrc-map-add-{priority}").into()),
        words,
        None,
        open,
        view,
        menu,
        move |this, index, cx| {
            if let Some(word) = owned.get(index).cloned() {
                // The first unused value, which the row below then changes. A picker that asked
                // two questions before it did anything would be two menus deep for one mapping.
                let value = if priority {
                    Priority::all()
                        .into_iter()
                        .map(|p| p.label().unwrap_or("normal").to_string())
                        .next()
                } else {
                    Kind::all()
                        .into_iter()
                        .map(|k| k.label().to_string())
                        .next()
                };
                this.map_task_word(word, value, priority, cx);
            }
        },
    ));

    setting_row(
        label,
        "A word the provider used, mapped onto one of Ubiq's. Click a mapping's \u{d7} to undo it.",
        div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap_2()
            .children(children)
            .into_any_element(),
    )
}

fn direction_row(app: &AppState, view: &Entity<AppState>, caps: ProviderCaps) -> AnyElement {
    let Some(draft) = app.tasksrc.draft.as_ref() else {
        return div().into_any_element();
    };
    // Two-way is offered where the provider accepts **any** write at all. That question is asked
    // of the ability table, not of the flags: `any` over a list of `fn`s is what a match over
    // eleven booleans would otherwise be.
    let writable = ABILITIES.iter().any(|ability| (ability.needs)(&caps));
    let two_way = draft.direction == Direction::TwoWay;

    setting_row(
        "Direction",
        "Pull is the default and stays the default. A binding that silently started writing to \
         somebody's shared board is the failure this default exists to prevent.",
        if writable {
            div()
                .flex()
                .gap_1p5()
                .child(toggle_pill(
                    "tasksrc-dir-pull",
                    "Pull only",
                    theme::accent(),
                    !two_way,
                    click(view, |this, _, cx| {
                        this.set_task_direction(Direction::Pull, cx)
                    }),
                ))
                .child(toggle_pill(
                    "tasksrc-dir-two",
                    "Two-way",
                    theme::accent(),
                    two_way,
                    click(view, |this, _, cx| {
                        this.set_task_direction(Direction::TwoWay, cx)
                    }),
                ))
                .into_any_element()
        } else {
            greyed("Pull only \u{2014} this provider accepts no write")
        },
    )
}

fn authority_row(app: &AppState, view: &Entity<AppState>, caps: ProviderCaps) -> AnyElement {
    let Some(draft) = app.tasksrc.draft.as_ref() else {
        return div().into_any_element();
    };
    let ubiq_wins = draft.authority == Authority::UbiqWins;
    let row = setting_row(
        "When both sides changed",
        "A switch, not a merge, and consulted only when both sides changed the same field. The \
         loser's value is kept in a comment.",
        div()
            .flex()
            .gap_1p5()
            .child(toggle_pill(
                "tasksrc-auth-remote",
                "The remote wins",
                theme::accent(),
                !ubiq_wins,
                click(view, |this, _, cx| {
                    this.set_task_authority(Authority::RemoteWins, cx)
                }),
            ))
            .child(toggle_pill(
                "tasksrc-auth-ubiq",
                "Ubiq wins",
                theme::accent(),
                ubiq_wins,
                click(view, |this, _, cx| {
                    this.set_task_authority(Authority::UbiqWins, cx)
                }),
            ))
            .into_any_element(),
    );

    let caveats: Vec<AnyElement> = CAVEATS
        .iter()
        .filter(|c| c.at == "authority" && (c.when)(&caps))
        .map(|c| note(c.note))
        .collect();
    if caveats.is_empty() {
        return row;
    }
    div()
        .w(relative(1.))
        .flex()
        .flex_col()
        .child(row)
        .children(caveats)
        .into_any_element()
}

fn interval_row(app: &AppState, view: &Entity<AppState>, caps: ProviderCaps) -> AnyElement {
    let Some(draft) = app.tasksrc.draft.as_ref() else {
        return div().into_any_element();
    };
    let asked = draft.poll;
    let actual = draft.poll_seconds();
    let floored = actual != asked;

    let row = setting_row(
        "Interval",
        "Seconds between passes. A floor applies, because a pass spends a rate-limit slot per \
         linked card.",
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(crate::ui::kit::stepper(
                "tasksrc-poll",
                format!("{actual}s"),
                click(view, |this, _, cx| this.nudge_task_poll(-60, cx)),
                click(view, |this, _, cx| this.nudge_task_poll(60, cx)),
            ))
            .when(floored, |this| {
                this.child(mono(
                    format!("asked for {asked}s, floored at {POLL_FLOOR}s"),
                    theme::text_faint(),
                ))
            })
            .into_any_element(),
    );

    let caveats: Vec<AnyElement> = CAVEATS
        .iter()
        .filter(|c| c.at == "interval" && (c.when)(&caps))
        .map(|c| note(c.note))
        .collect();
    if caveats.is_empty() {
        return row;
    }
    div()
        .w(relative(1.))
        .flex()
        .flex_col()
        .child(row)
        .children(caveats)
        .into_any_element()
}

fn auto_import_row(app: &AppState, view: &Entity<AppState>) -> AnyElement {
    let Some(draft) = app.tasksrc.draft.as_ref() else {
        return div().into_any_element();
    };
    let on = draft.auto_import;
    setting_row(
        "Import everything the filter matches",
        "Off, because import is explicit: the filter governs what is offered, not what is \
         created. Turn it on and every match becomes a task without being picked.",
        check_box(
            "tasksrc-auto-import",
            on,
            click(view, move |this, _, cx| this.set_task_auto_import(!on, cx)),
        )
        .into_any_element(),
    )
}

fn provider_row(app: &AppState, view: &Entity<AppState>) -> AnyElement {
    let src = &app.tasksrc;
    let labels: Vec<SharedString> = std::iter::once(SharedString::from("Not bound"))
        .chain(
            src.providers
                .iter()
                .map(|info| SharedString::from(info.label)),
        )
        .collect();
    let selected = src
        .draft
        .as_ref()
        .and_then(|draft| src.providers.iter().position(|i| i.id == draft.provider))
        .map(|ix| ix + 1)
        .or(Some(0));

    setting_row(
        "Provider",
        "What answers for this board. Which providers are offered is what this build ships with \
         \u{2014} a second edition adds its own and this page needs no change to draw it.",
        picker(
            "tasksrc-provider",
            labels,
            selected,
            src.menu.as_ref() == Some(&TaskSrcMenu::Provider),
            view,
            TaskSrcMenu::Provider,
            |this, index, cx| this.pick_task_provider(index.checked_sub(1), cx),
        ),
    )
}

/// Which identity this project calls the tracker as (`D189`).
///
/// **Not a per-provider branch.** The connections offered are the ones whose connector family the
/// provider *declared*, on the same footing as its schema and its capabilities, so this row names
/// no tracker. What it does branch on is the three honest states of that declaration, because they
/// are three different sentences and drawing one of them for all three would be a surface that
/// lies: a family with connections, a family with none yet, and no family at all.
fn connection_row(app: &AppState, view: &Entity<AppState>) -> AnyElement {
    let src = &app.tasksrc;
    let held = &app.workbench.settings.host.connections;
    let offered: Vec<&ubiq_proto::connectors::Connection> = src
        .provider()
        .map(|info| crate::state::tasksrc::connections_for(info, held))
        .unwrap_or_default();
    let labels: Vec<SharedString> = offered
        .iter()
        .map(|one| {
            let mut label = one.label.clone();
            if !one.account.is_empty() {
                label.push_str(" \u{2014} ");
                label.push_str(&one.account);
            }
            SharedString::from(label)
        })
        .collect();
    let selected = src
        .draft
        .as_ref()
        .and_then(|draft| offered.iter().position(|one| one.id == draft.connection));

    let control = if offered.is_empty() {
        greyed("No connection")
    } else {
        picker(
            "tasksrc-connection",
            labels,
            selected,
            src.menu.as_ref() == Some(&TaskSrcMenu::Connection),
            view,
            TaskSrcMenu::Connection,
            |this, index, cx| this.pick_task_connection(index, cx),
        )
    };

    let row = setting_row(
        "Connection",
        "Which identity this project calls the tracker as. The binding stores the reference and \
         never the token \u{2014} the credential stays in the connector family's own store.",
        control,
    );

    // The three states, said out loud. An empty picker with no reason beside it is the thing
    // somebody files a bug about.
    let sentence = match (
        src.provider().and_then(|info| info.connector),
        offered.is_empty(),
    ) {
        (None, _) => Some(
            "This build holds no connector family that can authenticate this provider, so there \
             is nothing to offer. The provider is listed, and a binding to it cannot be completed \
             until one exists.",
        ),
        (Some(_), true) => Some(
            "No connection for this provider yet. Settings \u{203a} Connections is where one is \
             made; it will appear here as soon as it is.",
        ),
        (Some(_), false) if !src.connection_held(held) => Some(
            "This binding names a connection this build does not hold \u{2014} one made on \
             another machine, or one since removed. Pick one above; nothing will sync until you \
             do.",
        ),
        _ => None,
    };

    match sentence {
        None => row,
        Some(text) => column(vec![row, note(text)]).into_any_element(),
    }
}

fn actions_row(app: &AppState, view: &Entity<AppState>) -> AnyElement {
    let src = &app.tasksrc;
    let can_save = src.dirty() && src.complete();
    div()
        .w(relative(1.))
        .flex()
        .items_center()
        .gap_2()
        .pt_4()
        .children(src.error.as_ref().map(|error| {
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::danger())
                .child(SharedString::from(error.clone()))
        }))
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "tasksrc-import",
            Some(IconName::ArrowDown),
            "Import\u{2026}",
            click(view, |this, _, cx| this.open_task_import(cx)),
        ))
        .child(ghost_button(
            "tasksrc-unbind",
            Some(IconName::Close),
            "Unbind",
            click(view, |this, _, cx| this.unbind_task_source(cx)),
        ))
        .child(if can_save {
            primary_button(
                "tasksrc-save",
                Some(IconName::Check),
                "Save binding",
                click(view, |this, _, cx| this.save_task_source(cx)),
            )
            .into_any_element()
        } else {
            greyed(if src.dirty() {
                "Fill every required field to save"
            } else {
                "Saved"
            })
        })
        .into_any_element()
}

// ── The import dialog ───────────────────────────────────────────────

/// What the filter currently offers, to tick.
///
/// `R12`: **nothing appears on the board that a person did not pick.** An item that already has a
/// link row is drawn and not tickable — a second copy of a task that exists is the exact failure
/// this list prevents.
pub fn import_dialog(
    app: &AppState,
    window: &Window,
    cx: &mut gpui::Context<AppState>,
) -> AnyElement {
    let Some(dialog) = app.tasksrc.import.as_ref() else {
        return div().into_any_element();
    };
    let view = cx.entity();

    let rows: Vec<AnyElement> = dialog
        .items
        .iter()
        .enumerate()
        .map(|(ix, item)| {
            let linked = dialog.is_linked(&item.id);
            let picked = dialog.is_picked(&item.id);
            let id = item.id.clone();
            div()
                .w(relative(1.))
                .flex()
                .items_center()
                .gap_2()
                .py_1()
                .child(if linked {
                    mono("linked", theme::text_faint()).into_any_element()
                } else {
                    check_box(
                        ElementId::Name(format!("tasksrc-import-{ix}").into()),
                        picked,
                        click(&view, move |this, _, cx| {
                            this.toggle_import_item(id.clone(), cx)
                        }),
                    )
                    .into_any_element()
                })
                .children(item.key.as_ref().map(|key| {
                    mono(key.clone(), theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(if linked {
                            theme::text_faint()
                        } else {
                            theme::text()
                        })
                        .child(SharedString::from(item.title.clone())),
                )
                .into_any_element()
        })
        .collect();

    let body = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(crate::ui::kit::modal_note(
            "Everything the binding's filter offers right now. Nothing below becomes a task until \
             it is ticked \u{2014} the filter governs what is offered, not what is created.",
        ))
        .when(dialog.loading, |this| {
            this.child(mono("fetching\u{2026}", theme::text_muted()))
        })
        .children(dialog.error.as_ref().map(|error| {
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::danger())
                .child(SharedString::from(error.clone()))
        }))
        .child(
            div()
                .id("tasksrc-import-list")
                .flex()
                .flex_col()
                .max_h(px(360.))
                .overflow_y_scroll()
                .children(rows),
        )
        .into_any_element();

    let picked = dialog.picked.len();
    let footer = div()
        .w(relative(1.))
        .flex()
        .items_center()
        .gap_2()
        .child(mono(
            format!("{picked} picked"),
            if picked == 0 {
                theme::text_faint()
            } else {
                theme::text_muted()
            },
        ))
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "tasksrc-import-cancel",
            None,
            "Cancel",
            click(&view, |this, _, cx| this.close_task_import(cx)),
        ))
        .child(primary_button(
            "tasksrc-import-go",
            Some(IconName::Check),
            "Import",
            click(&view, |this, _, cx| this.import_picked_items(cx)),
        ))
        .into_any_element();

    crate::ui::kit::modal(
        "tasksrc-import",
        theme::accent(),
        "Import from the board",
        body,
        footer,
        dismiss(&view, Layer::TaskImport, |this, _, cx| {
            this.close_task_import(cx)
        }),
        window,
    )
}

// ── The card badge ──────────────────────────────────────────────────

/// What happened to one card, when it is worth saying.
///
/// **`Parked` is the one this exists for.** M6 files a parked item's link row and nothing drew it;
/// a task whose remote lane is not in the lane map sits in the column it landed in, and without
/// this badge the only visible fact is that it did not move. `Linked` draws nothing at all: the
/// ordinary case is not news, and a dot on every synced card is a dot nobody reads.
pub fn sync_badge(app: &AppState, task: TaskId) -> Option<AnyElement> {
    let link = app.tasksrc.link(task)?;
    let state = link.state;
    if !state.is_notable() {
        return None;
    }
    let colour = link_colour(state);
    // The state's own sentence, and — where there is any — **what actually differs**. A badge that
    // says "drifted" and nothing else sends somebody to the settings page to find out in what.
    let tooltip = SharedString::from(if link.drift.is_empty() {
        state.note().to_string()
    } else {
        let fields: Vec<&str> = link
            .drift
            .iter()
            .map(|drift| drift.field.as_str())
            .collect();
        format!("{}\n\nDiffers in: {}.", state.note(), fields.join(", "))
    });

    Some(
        div()
            .id(ElementId::Name(format!("tasksrc-badge-{task}").into()))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(status_dot(colour, theme::pane_bg()))
            .child(mono(state.label(), colour).text_size(theme::font(Family::Chrome, Role::Micro)))
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .into_any_element(),
    )
}

/// The tone each link state wears, once, so the card's badge and the board's item agree.
///
/// Parked is a **warning**, not a failure: nothing broke, and a person has to decide whether to
/// bind the lane. Conflict is the danger tone because a write was refused.
fn link_colour(state: LinkState) -> gpui::Rgba {
    match state {
        LinkState::Linked => theme::success(),
        LinkState::Parked => theme::warning(),
        LinkState::Drifted => theme::info(),
        LinkState::Conflict => theme::danger(),
        LinkState::Unlinked => theme::text_faint(),
    }
}

// ── The board's status item ─────────────────────────────────────────

/// Whether the board as a whole is in step: the state, the last pass, the drift count, and the
/// force button.
///
/// Draws nothing when the project is unbound. A strip that said "not syncing" on every board would
/// be a permanent answer to a question nobody asked.
pub fn status_item(app: &AppState, cx: &mut gpui::Context<AppState>) -> Option<AnyElement> {
    let src = &app.tasksrc;
    if src.state == SyncState::Unbound {
        return None;
    }
    let view = cx.entity();
    let (colour, word) = match src.state {
        SyncState::Unbound => (theme::text_faint(), "unbound"),
        SyncState::Idle => (theme::success(), "in step"),
        SyncState::Running => (theme::info(), "syncing\u{2026}"),
        SyncState::Failed => (theme::danger(), "sync failed"),
    };
    let drifted = src.drifted.len();
    let last = src
        .last_sync
        .map(|at| at.format("%H:%M").to_string())
        .unwrap_or_else(|| "never".to_string());
    let tooltip = SharedString::from(match src.error.as_deref() {
        Some(error) => error.to_string(),
        None => format!("Last pass {last}. Click to run one now."),
    });

    Some(
        div()
            .id("tasksrc-status")
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .cursor_pointer()
            .child(status_dot(colour, theme::pane_bg()))
            .child(mono(word, colour))
            .when(drifted > 0, |this| {
                this.child(mono(format!("{drifted} drifted"), theme::info()))
            })
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .on_click(window_click(&view))
            .into_any_element(),
    )
}

fn window_click(
    view: &Entity<AppState>,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |_, window, cx| {
        view.update(cx, |this, cx| this.sync_task_source_now(window, cx));
    }
}

// ── Small shared pieces ─────────────────────────────────────────────

/// A dropdown, with the section's own open-menu discipline folded in so no caller repeats it.
#[allow(clippy::too_many_arguments)]
fn picker(
    id: impl Into<ElementId>,
    items: Vec<SharedString>,
    selected: Option<usize>,
    open: bool,
    view: &Entity<AppState>,
    menu: TaskSrcMenu,
    on_pick: impl Fn(&mut AppState, usize, &mut gpui::Context<AppState>) + 'static,
) -> AnyElement {
    let label = selected
        .and_then(|ix| items.get(ix).cloned())
        .unwrap_or_else(|| SharedString::from(UNSET));
    let open_menu = menu.clone();
    let mut control = Picker::new(id, label)
        .items(items)
        .style(PickerStyle::Field)
        .open(open)
        .on_toggle(handler(view, move |this, _, cx| {
            this.open_tasksrc_menu(open_menu.clone(), cx)
        }))
        .on_dismiss(handler(view, |this, _, cx| this.close_menu(cx)))
        .on_pick(indexed(view, move |this, index, _, cx| {
            this.close_menu(cx);
            on_pick(this, index, cx);
        }));
    if let Some(ix) = selected {
        control = control.selected(ix);
    }
    control.into_any_element()
}

/// A row of the lane map: the remote's own word, a dot in the colour it maps to, and the picker.
fn map_row(name: String, dot: Option<gpui::Rgba>, control: AnyElement) -> AnyElement {
    div()
        .w(relative(1.))
        .py_1p5()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .min_w(px(0.))
                .child(status_dot(
                    dot.unwrap_or_else(theme::text_faint),
                    theme::pane_bg(),
                ))
                .child(
                    div()
                        .truncate()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(SharedString::from(name)),
                ),
        )
        .child(control)
        .into_any_element()
}

fn sub_heading(title: &str, note: &str) -> AnyElement {
    div()
        .w(relative(1.))
        .pt_4()
        .pb_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_strong())
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

fn note(text: &str) -> AnyElement {
    div()
        .w(relative(1.))
        .py_1()
        .max_w(px(560.))
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_faint())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

/// A control that is drawn and does nothing, with the reason in its place.
///
/// The section never *removes* a control a provider cannot drive: two providers' pages are the
/// same shape, and the difference between them is a legible sentence rather than a missing row.
fn greyed(reason: &str) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .border_1()
        .border_color(theme::border())
        .child(mono(reason.to_string(), theme::text_faint()))
        .into_any_element()
}

/// The glyph the rail and the status item share for this feature.
pub fn mark() -> UbiqIcon {
    UbiqIcon::ModeTasks
}

/// Which project the section is about, for a caller that has to ask before drawing.
pub fn bound_project(app: &AppState) -> Option<ProjectId> {
    app.tasksrc.project
}

/// The menu id every dropdown in this module opens under.
pub const MENU: MenuId = MenuId::TaskSrc;
