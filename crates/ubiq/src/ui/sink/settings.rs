//! Application settings, composed from the kit: a nav, setting rows, and a harness accordion.
//!
//! **Nothing here is a new primitive.** The page is how those primitives sit together as a
//! settings screen — a left nav of selectable rows, a heading plus a column of label/control
//! rows, radio cards, and harnesses that open with `slab` the way every other surface does. The
//! values are fixtures on [`crate::state::sink::SettingsDemo`]; a control that cannot hold a
//! value is not being tested.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_component::input::{Input, Textarea};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::sink::{
    AUTH_CHOICES, DENSITY_CHOICES, HARNESS_FIXTURES, MODE_CHOICES, MODEL_CHOICES, PERMISSIONS,
    SettingsMenu, SettingsNav, THEME_CHOICES, THINKING_CHOICES,
};
use crate::theme;
use crate::ui::kit::{
    Picker, PickerStyle, card, check_box, choice_pill, ghost_button, icon_button, meter, mono,
    pill, primary_button, slab, status_dot, stepper,
};
use crate::ui::sink::style::{framed_active, input_on, textarea_on};
use crate::ui::{handler, indexed};

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id("sink-settings")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(nav(app, window, cx))
        .child(body(app, window, cx))
        .into_any_element()
}

fn nav(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let query = app.sink_search.read(cx).value().to_lowercase();
    let current = app.sink.settings.nav;

    let items: Vec<AnyElement> = SettingsNav::all()
        .iter()
        .copied()
        .filter(|item| query.is_empty() || item.label().to_lowercase().contains(&query))
        .map(|item| {
            let count = match item {
                SettingsNav::Harnesses => Some(HARNESS_FIXTURES.len()),
                _ => None,
            };
            nav_item(
                ElementId::Name(format!("sink-settings-nav-{}", item.label()).into()),
                nav_icon(item),
                item.label(),
                count,
                item == current,
                cx.listener(move |this, _, _, cx| this.set_sink_settings_nav(item, cx)),
            )
        })
        .collect();

    div()
        .id("sink-settings-nav")
        .w(px(220.))
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .px_2()
        .py_3()
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .child(
            framed_active(theme::border(), input_on(&app.sink_search, window, cx))
                .h(px(30.))
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Search)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(Input::new(&app.sink_search).appearance(false)),
        )
        .children(items)
        .into_any_element()
}

fn nav_icon(item: SettingsNav) -> IconName {
    match item {
        SettingsNav::Appearance => IconName::Palette,
        SettingsNav::Harnesses => IconName::Asterisk,
        SettingsNav::AgentDefaults => IconName::Cpu,
        SettingsNav::Sessions => IconName::Folder,
        SettingsNav::Keyboard => IconName::Menu,
        SettingsNav::Privacy => IconName::Inspector,
    }
}

fn body(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.sink.settings.nav {
        SettingsNav::Appearance => appearance(app, cx),
        SettingsNav::Harnesses => harnesses(app, window, cx),
        SettingsNav::AgentDefaults => defaults(app, cx),
        SettingsNav::Sessions => sessions(),
        SettingsNav::Keyboard => keyboard(),
        SettingsNav::Privacy => privacy(),
    };

    div()
        .id("sink-settings-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_6()
        .py_5()
        .child(content)
        .into_any_element()
}

fn appearance(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let settings = &app.sink.settings;

    let theme_pills: Vec<AnyElement> = THEME_CHOICES
        .iter()
        .enumerate()
        .map(|(index, label)| {
            choice_pill(
                ElementId::Name(format!("sink-settings-theme-{index}").into()),
                *label,
                settings.theme == index,
                cx.listener(move |this, _, _, cx| this.set_sink_settings_theme(index, cx)),
            )
            .into_any_element()
        })
        .collect();

    let density_pills: Vec<AnyElement> = DENSITY_CHOICES
        .iter()
        .enumerate()
        .map(|(index, label)| {
            choice_pill(
                ElementId::Name(format!("sink-settings-density-{index}").into()),
                *label,
                settings.density == index,
                cx.listener(move |this, _, _, cx| this.set_sink_settings_density(index, cx)),
            )
            .into_any_element()
        })
        .collect();

    column(vec![
        heading(
            "Appearance",
            "How the workbench looks. These apply to every project on this machine.",
        ),
        setting_row(
            "Theme",
            "Ubiq follows the operating system unless you pin a theme here.",
            div()
                .flex()
                .gap_1()
                .children(theme_pills)
                .into_any_element(),
        ),
        setting_row(
            "Accent follows the project",
            "Each project keeps its own color. The title chip, active rails and focus rings take it.",
            check_box(
                "sink-settings-accent",
                settings.accent_follows,
                cx.listener(|this, _, _, cx| this.toggle_sink_accent_follows(cx)),
            )
            .into_any_element(),
        ),
        setting_row(
            "Interface density",
            "Compact tightens the explorer, tabs and lists by about a fifth.",
            div()
                .flex()
                .gap_1()
                .children(density_pills)
                .into_any_element(),
        ),
        setting_row(
            "Editor font size",
            "Applies to the editor and both terminal panes.",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(stepper(
                    "sink-settings-font",
                    format!("{}", settings.font_size),
                    cx.listener(|this, _, _, cx| this.nudge_sink_font(-1, cx)),
                    cx.listener(|this, _, _, cx| this.nudge_sink_font(1, cx)),
                ))
                .child(mono("px", theme::text_faint()).text_size(px(11.)))
                .into_any_element(),
        ),
        setting_row(
            "Reduce motion",
            "Stops the state pulses and graph transitions. Follows the OS setting by default.",
            check_box(
                "sink-settings-motion",
                settings.reduce_motion,
                cx.listener(|this, _, _, cx| this.toggle_sink_reduce_motion(cx)),
            )
            .into_any_element(),
        ),
    ])
}

fn harnesses(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let settings = &app.sink.settings;
    let enabled = settings.enabled_count();

    let cards: Vec<AnyElement> = HARNESS_FIXTURES
        .iter()
        .enumerate()
        .map(|(index, fixture)| harness_card(app, index, fixture, window, cx))
        .collect();

    column(vec![
        heading(
            "Harnesses",
            "Every agent runs on a harness. Register as many as you like — the same tool twice \
             with different credentials is normal, and each entry carries its own defaults.",
        ),
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(primary_button(
                "sink-settings-add-harness",
                Some(IconName::Plus),
                "Add harness",
                |_, _, _| {},
            ))
            .child(
                mono(
                    format!(
                        "{} registered · {} enabled",
                        HARNESS_FIXTURES.len(),
                        enabled
                    ),
                    theme::text_faint(),
                )
                .text_size(px(11.)),
            )
            .into_any_element(),
        div()
            .px_3()
            .py_2()
            .bg(theme::warning_soft())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::warning())
            .text_size(px(12.5))
            .text_color(theme::text())
            .child(SharedString::from(
                "Secrets live in the OS keychain. Ubiq stores only a reference in settings.json, \
                 so this file is safe to sync.",
            ))
            .into_any_element(),
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(cards)
            .into_any_element(),
    ])
}

fn harness_card(
    app: &AppState,
    index: usize,
    fixture: &crate::state::sink::HarnessFixture,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let settings = &app.sink.settings;
    let open = settings.open_harness == Some(index);
    let on = settings.harness_on[index];
    let edge = if fixture.connected {
        theme::success()
    } else {
        theme::border()
    };

    let mut body = slab(edge).id(ElementId::Name(format!("sink-harness-{index}").into()));

    let header = div()
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id(ElementId::Name(format!("sink-harness-head-{index}").into()))
                .flex()
                .flex_1()
                .min_w(px(0.))
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(Size::XSmall)
                    .text_color(theme::text_faint()),
                )
                .child(
                    div()
                        .size(px(22.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .bg(theme::surface_raised())
                        .child(
                            Icon::new(IconName::Asterisk)
                                .with_size(Size::XSmall)
                                .text_color(theme::accent()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child(SharedString::from(fixture.name)),
                        )
                        .child(
                            mono(
                                format!(
                                    "{} · {} · {} · {}",
                                    fixture.kind, fixture.auth, fixture.account, fixture.model
                                ),
                                theme::text_faint(),
                            )
                            .text_size(px(11.)),
                        ),
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_sink_harness_open(index, cx)),
                ),
        )
        .when(fixture.connected, |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(status_dot(theme::success(), theme::success_soft()))
                    .child(mono("connected", theme::success()).text_size(px(11.))),
            )
        })
        .child(check_box(
            ElementId::Name(format!("sink-harness-on-{index}").into()),
            on,
            cx.listener(move |this, _, _, cx| this.toggle_sink_harness(index, cx)),
        ));

    body = body.child(header);
    if open {
        body = body.child(harness_form(app, window, cx));
    }
    body.into_any_element()
}

fn harness_form(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let settings = &app.sink.settings;
    let view = cx.entity();
    let open_menu = app.workbench.open_menu == Some(MenuId::SinkSettings);

    div()
        .flex()
        .flex_col()
        .px_3()
        .pb_3()
        .child(form_row(
            "Display name",
            "How this harness appears in agent pickers.",
            framed_active(
                theme::border(),
                input_on(&app.sink_harness_name, window, cx),
            )
            .w(px(240.))
            .h(px(30.))
            .items_center()
            .child(Input::new(&app.sink_harness_name).appearance(false))
            .into_any_element(),
        ))
        .child(form_row(
            "Executable",
            "Absolute path. Ubiq never uses your shell PATH.",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    framed_active(
                        theme::border(),
                        input_on(&app.sink_harness_exec, window, cx),
                    )
                    .w(px(240.))
                    .h(px(30.))
                    .items_center()
                    .child(Input::new(&app.sink_harness_exec).appearance(false)),
                )
                .child(ghost_button(
                    "sink-harness-browse",
                    None,
                    "Browse\u{2026}",
                    |_, _, _| {},
                ))
                .into_any_element(),
        ))
        .child(form_row(
            "Authentication",
            "Two entries of the same tool may use different accounts.",
            Picker::new("sink-harness-auth", AUTH_CHOICES[settings.auth])
                .items(AUTH_CHOICES)
                .selected(settings.auth)
                .style(PickerStyle::Chip)
                .open(open_menu && settings.menu == Some(SettingsMenu::Auth))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_sink_settings_menu(SettingsMenu::Auth, cx)
                }))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.pick_sink_settings_menu(index, cx)
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
                .into_any_element(),
        ))
        .child(form_row(
            "Account",
            "Signed in through the provider. Re-run the flow to switch account.",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(mono(HARNESS_FIXTURES[0].account, theme::text()).text_size(px(12.5)))
                .child(ghost_button(
                    "sink-harness-reauth",
                    None,
                    "Re-authenticate",
                    |_, _, _| {},
                ))
                .into_any_element(),
        ))
        .child(form_row(
            "Default model",
            "Agents start here; each chat can change it.",
            Picker::new("sink-harness-model", MODEL_CHOICES[settings.model])
                .items(MODEL_CHOICES)
                .selected(settings.model)
                .style(PickerStyle::Chip)
                .open(open_menu && settings.menu == Some(SettingsMenu::Model))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_sink_settings_menu(SettingsMenu::Model, cx)
                }))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.pick_sink_settings_menu(index, cx)
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
                .into_any_element(),
        ))
        .child(form_row(
            "Default thinking & mode",
            "Starting budget and permissions for new agents.",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Picker::new("sink-harness-thinking", THINKING_CHOICES[settings.thinking])
                        .items(THINKING_CHOICES)
                        .selected(settings.thinking)
                        .style(PickerStyle::Chip)
                        .open(open_menu && settings.menu == Some(SettingsMenu::Thinking))
                        .on_toggle(handler(&view, |this, _, cx| {
                            this.open_sink_settings_menu(SettingsMenu::Thinking, cx)
                        }))
                        .on_pick(indexed(&view, |this, index, _, cx| {
                            this.pick_sink_settings_menu(index, cx)
                        }))
                        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
                )
                .child(
                    Picker::new("sink-harness-mode", MODE_CHOICES[settings.mode])
                        .items(MODE_CHOICES)
                        .selected(settings.mode)
                        .style(PickerStyle::Chip)
                        .open(open_menu && settings.menu == Some(SettingsMenu::Mode))
                        .on_toggle(handler(&view, |this, _, cx| {
                            this.open_sink_settings_menu(SettingsMenu::Mode, cx)
                        }))
                        .on_pick(indexed(&view, |this, index, _, cx| {
                            this.pick_sink_settings_menu(index, cx)
                        }))
                        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
                )
                .into_any_element(),
        ))
        .child(
            div()
                .py_3()
                .flex()
                .flex_col()
                .gap_1p5()
                .border_t_1()
                .border_color(theme::border())
                .child(label_block(
                    "Initial prompt",
                    "Prepended to every agent on this harness, before the task. Keep it about \
                     house rules, not about one task.",
                ))
                .child(
                    framed_active(
                        theme::border(),
                        textarea_on(&app.sink_harness_prompt, window, cx),
                    )
                    .p_2()
                    .child(
                        Textarea::new(&app.sink_harness_prompt)
                            .appearance(false)
                            .bordered(false)
                            .w_full()
                            .text_size(px(13.)),
                    ),
                ),
        )
        .child(form_row(
            "Environment",
            "Extra variables for the process. Press Enter to add.",
            env_chips(app, window, cx),
        ))
        .into_any_element()
}

fn env_chips(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let chips: Vec<AnyElement> = app
        .sink
        .settings
        .env
        .iter()
        .enumerate()
        .map(|(index, pair)| {
            pill(theme::border())
                .gap_1()
                .child(mono(pair.clone(), theme::text()).text_size(px(11.)))
                .child(
                    div()
                        .id(ElementId::Name(format!("sink-env-drop-{index}").into()))
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|this| this.bg(theme::hover()))
                        .child(
                            Icon::new(IconName::Close)
                                .with_size(Size::XSmall)
                                .text_color(theme::text_faint()),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove_sink_env(index, cx)),
                        ),
                )
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .items_center()
        .gap_2()
        .flex_wrap()
        .children(chips)
        .child(
            framed_active(theme::border(), input_on(&app.sink_harness_env, window, cx))
                .w(px(140.))
                .h(px(26.))
                .items_center()
                .child(Input::new(&app.sink_harness_env).appearance(false)),
        )
        .child(icon_button(
            "sink-env-add",
            IconName::Plus,
            false,
            cx.listener(|this, _, window, cx| this.add_sink_env(window, cx)),
        ))
        .into_any_element()
}

fn defaults(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let settings = &app.sink.settings;

    let modes: Vec<AnyElement> = PERMISSIONS
        .iter()
        .enumerate()
        .map(|(index, (label, note))| {
            let selected = settings.permission == index;
            card(
                ElementId::Name(format!("sink-permission-{index}").into()),
                if selected {
                    theme::accent()
                } else {
                    theme::border()
                },
                selected,
            )
            .px_3()
            .py_2()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_dot(
                        if selected {
                            theme::accent()
                        } else {
                            theme::text_faint()
                        },
                        if selected {
                            theme::accent_soft()
                        } else {
                            theme::surface()
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .text_color(theme::text())
                                    .child(SharedString::from(*label)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme::text_muted())
                                    .child(SharedString::from(*note)),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_sink_permission(index, cx)))
            .into_any_element()
        })
        .collect();

    column(vec![
        heading(
            "Agent defaults",
            "Applied to every new agent. A task or a single agent can override any of them.",
        ),
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label_block(
                "Default permission mode",
                "What an agent may do before it asks you.",
            ))
            .child(div().flex().flex_col().gap_2().children(modes))
            .into_any_element(),
        setting_row(
            "Maximum concurrent agents",
            "Across every session in this window. Coordinators count too.",
            div()
                .flex()
                .items_center()
                .gap_3()
                .w(px(280.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(meter(settings.max_agents as f32 / 16.0, theme::accent())),
                )
                .child(stepper(
                    "sink-settings-agents",
                    format!("{} agents", settings.max_agents),
                    cx.listener(|this, _, _, cx| this.nudge_sink_agents(-1, cx)),
                    cx.listener(|this, _, _, cx| this.nudge_sink_agents(1, cx)),
                ))
                .into_any_element(),
        ),
        setting_row(
            "Context warning threshold",
            "The context ring turns amber past this share of the window.",
            div()
                .flex()
                .items_center()
                .gap_3()
                .w(px(280.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(meter(settings.context_warn as f32 / 100.0, theme::accent())),
                )
                .child(stepper(
                    "sink-settings-warn",
                    format!("{}%", settings.context_warn),
                    cx.listener(|this, _, _, cx| this.nudge_sink_warn(-5, cx)),
                    cx.listener(|this, _, _, cx| this.nudge_sink_warn(5, cx)),
                ))
                .into_any_element(),
        ),
        setting_row(
            "Retry once after a tool error",
            "A second failure stops the agent and marks the sub-task blocked.",
            check_box(
                "sink-settings-retry",
                settings.retry,
                cx.listener(|this, _, _, cx| this.toggle_sink_retry(cx)),
            )
            .into_any_element(),
        ),
        setting_row(
            "Idle timeout",
            "How long a waiting agent sits before it is marked idle.",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(stepper(
                    "sink-settings-idle",
                    format!("{}", settings.idle),
                    cx.listener(|this, _, _, cx| this.nudge_sink_idle(-5, cx)),
                    cx.listener(|this, _, _, cx| this.nudge_sink_idle(5, cx)),
                ))
                .child(mono("min", theme::text_faint()).text_size(px(11.)))
                .into_any_element(),
        ),
    ])
}

fn sessions() -> AnyElement {
    column(vec![
        heading(
            "Sessions & worktrees",
            "Where a session keeps its folder, and whether a new one gets a worktree of its own.",
        ),
        setting_row(
            "Create a worktree for each session",
            "The session's folder is a git worktree, so two sessions never share a dirty tree.",
            check_box("sink-settings-worktree", true, |_, _, _| {}).into_any_element(),
        ),
        setting_row(
            "Restore the last session on open",
            "A window that closed with a session open comes back to it.",
            check_box("sink-settings-restore", true, |_, _, _| {}).into_any_element(),
        ),
    ])
}

fn keyboard() -> AnyElement {
    column(vec![
        heading(
            "Keyboard",
            "Bindings the workbench owns. A harness keeps its own, inside its pane.",
        ),
        setting_row(
            "Command palette",
            "Search commands, files and settings from the titlebar.",
            mono("cmd-k", theme::text_muted())
                .text_size(px(12.5))
                .into_any_element(),
        ),
        setting_row(
            "Toggle the sink",
            "The application's own test bench, this page.",
            mono("cmd-shift-s", theme::text_muted())
                .text_size(px(12.5))
                .into_any_element(),
        ),
    ])
}

fn privacy() -> AnyElement {
    column(vec![
        heading(
            "Privacy & data",
            "What Ubiq writes down, and what it sends nowhere.",
        ),
        setting_row(
            "Keep a local log",
            "The console reads it. Nothing leaves this machine.",
            check_box("sink-settings-log", true, |_, _, _| {}).into_any_element(),
        ),
        setting_row(
            "Crash reports",
            "Off. A report would have to name a folder, and this page does not.",
            check_box("sink-settings-crash", false, |_, _, _| {}).into_any_element(),
        ),
    ])
}

// ── Furniture the two settings pages share ──────────────────────────

pub(super) fn heading(title: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pb_2()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

pub(super) fn setting_row(label: &str, note: &str, control: AnyElement) -> AnyElement {
    div()
        .w(relative(1.))
        .py_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .border_b_1()
        .border_color(theme::border())
        .child(label_block(label, note))
        .child(control)
        .into_any_element()
}

pub(super) fn label_block(label: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .text_size(px(13.5))
                .text_color(theme::text())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

fn form_row(label: &str, note: &str, control: AnyElement) -> AnyElement {
    setting_row(label, note, control)
}

pub(super) fn nav_item(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &str,
    count: Option<usize>,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let fg = if selected {
        theme::text()
    } else {
        theme::text_muted()
    };
    let icon_fg = if selected {
        theme::accent()
    } else {
        theme::text_muted()
    };

    let mut row = div()
        .id(id)
        .h(px(32.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(Icon::new(icon).with_size(Size::Small).text_color(icon_fg))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.5))
                .text_color(fg)
                .child(SharedString::from(label.to_string())),
        );

    if let Some(count) = count {
        row = row.child(mono(format!("{count}"), theme::text_faint()).text_size(px(11.)));
    }
    if selected {
        row = row
            .bg(theme::accent_soft())
            .border_l(px(theme::ACCENT_EDGE))
            .border_color(theme::accent());
    }

    row.on_click(on_click).into_any_element()
}

fn column(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w(relative(1.))
        .children(children)
        .into_any_element()
}
