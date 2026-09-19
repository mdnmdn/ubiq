//! The size controls, drawn once and used twice.
//!
//! The status bar's popover and the Size settings section offer the *same* two sliders and the
//! *same* row of preset pills. They are built here rather than written out in both places — the
//! discipline `_docs/inbox/component-reuse-proposal.md` argues for, and the reason a change to
//! either control is one edit.
//!
//! **No number, no unit and no `px` appears in any of it.** The old status-bar dropdown said
//! `13 px` and moved four unrelated surfaces; what replaced it says *smaller* and *larger* with
//! two icons and a track, because a size is chosen by eye. The pills carry names, which are the
//! only words on the panel.

use std::rc::Rc;

use gpui::{
    AnyElement, Context, ElementId, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, px,
};

use crate::app::AppState;
use crate::state::prefs::SizePreset;
use crate::state::{MenuId, SizePrompt};
use crate::theme;
use crate::ui::handler;
use crate::ui::kit::{
    Slider, UbiqIcon, choice_pill, ghost_button, popover, prompt_modal, section_label,
};

/// How wide the popover draws. Wide enough that a slider has a track worth dragging and the four
/// preset names sit on one line at the default scale.
const PANEL_WIDTH: f32 = 260.;

/// The row of preset pills, one lit — or none, when the axes spell no preset and the trigger
/// reads *Custom*.
///
/// `prefix` namespaces the element ids, because the popover and the settings page can both be on
/// screen and a duplicated id is a row that answers for the other one.
pub fn preset_pills(
    app: &AppState,
    prefix: &'static str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let active = app.active_size_preset().map(|preset| preset.name);
    let pills: Vec<AnyElement> = app
        .size_presets()
        .into_iter()
        .map(|preset| {
            let lit = active.as_deref() == Some(preset.name.as_str());
            let label: SharedString = preset.name.clone().into();
            choice_pill(
                ElementId::Name(format!("{prefix}-preset-{}", preset.name).into()),
                label,
                lit,
                cx.listener(move |this, _, window, cx| this.apply_size_preset(&preset, window, cx)),
            )
            .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .children(pills)
        .into_any_element()
}

/// The UI-scale slider: every dimension in the window, small window to large window.
pub fn interface_slider(app: &AppState, id: &'static str) -> AnyElement {
    Slider::new(
        id,
        &app.ui_scale_slider,
        "How big the whole window is drawn \u{2014} every row, every icon and the text with them",
    )
    .leading(UbiqIcon::SizeInterfaceSmall)
    .trailing(UbiqIcon::SizeInterfaceLarge)
    .into_any_element()
}

/// The text-ratio slider: type only, within that window.
pub fn text_slider(app: &AppState, id: &'static str) -> AnyElement {
    Slider::new(
        id,
        &app.text_ratio_slider,
        "How big text is drawn inside that window \u{2014} denser or airier type, same furniture",
    )
    .leading(UbiqIcon::SizeTextSmall)
    .trailing(UbiqIcon::SizeTextLarge)
    .into_any_element()
}

/// What the popover's trigger says on hover: the preset the axes spell, or *Custom*.
pub fn trigger_tooltip(app: &AppState) -> String {
    match app.active_size_preset() {
        Some(preset) => format!("Size \u{2014} {}", preset.name),
        None => "Size \u{2014} Custom".to_string(),
    }
}

/// The anchored panel the status bar's trigger opens.
///
/// **Not a modal**: no scrim, nothing behind it is occluded, and it is peeled by the same Escape
/// and the same outside click every other menu is — it is `MenuId::Size`, so `cancel_dialog`
/// already has it and it needs no rung of its own. It is drawn upward, because the strip it hangs
/// from is the bottom of the window.
pub fn panel(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();

    popover(
        ElementId::Name("size-popover".into()),
        px(PANEL_WIDTH),
        Some("size-popover"),
        Some(Rc::new(handler(&view, |this, _, cx| this.close_menu(cx)))),
        vec![
            div()
                .px_2()
                .pt_1()
                .child(section_label("presets"))
                .into_any_element(),
            div()
                .px_2()
                .pb_1()
                .child(preset_pills(app, "size-popover", cx))
                .into_any_element(),
            div()
                .px_2()
                .py_1()
                .child(interface_slider(app, "size-popover-interface"))
                .into_any_element(),
            div()
                .px_2()
                .py_1()
                .child(text_slider(app, "size-popover-text"))
                .into_any_element(),
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_1()
                .pt_1()
                .child(ghost_button(
                    "size-popover-save",
                    None,
                    "Save preset\u{2026}",
                    cx.listener(|this, _, window, cx| this.ask_size_preset_name(window, cx)),
                ))
                .child(ghost_button(
                    "size-popover-reset",
                    None,
                    "Reset",
                    cx.listener(|this, _, window, cx| this.reset_size(window, cx)),
                ))
                .into_any_element(),
        ],
    )
}

/// Whether the popover is up.
pub fn is_open(app: &AppState) -> bool {
    app.workbench.open_menu == Some(MenuId::Size)
}

/// The name prompt both the popover's *Save preset…* and the settings list's rename raise.
///
/// `kit::prompt_modal` rather than a hand-rolled field, which is the rule for every naming
/// question in the window.
pub fn name_prompt(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(prompt) = app.workbench.size_prompt.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let value = app.size_name_input.read(cx).value().to_string();
    let enabled = crate::app::size_name_valid(&value, &prompt);
    let (title, note, confirm) = match &prompt {
        SizePrompt::Save => (
            "Save size preset",
            "The two sliders as they stand, under a name. A name already in use is replaced \
             rather than doubled, and a preset carries no palette \u{2014} only the two sizes.",
            "Save",
        ),
        SizePrompt::Rename { .. } => (
            "Rename size preset",
            "The preset keeps both its sizes; only what it is called changes.",
            "Rename",
        ),
    };

    prompt_modal(
        "size-preset-name",
        title,
        Some(note),
        "Name",
        &app.size_name_input,
        confirm,
        enabled,
        handler(&view, |this, _, cx| this.confirm_size_prompt(cx)),
        handler(&view, |this, _, cx| this.close_size_prompt(cx)),
        window,
        cx,
    )
}

/// One saved preset's row in the Size settings section: what it is called, what it sets, and the
/// two things a popover has no room for.
pub fn preset_row(app: &AppState, preset: &SizePreset, cx: &mut Context<AppState>) -> AnyElement {
    let name = preset.name.clone();
    let saved = app.size_preset_is_saved(&name);
    let lit = app.active_size_preset().map(|active| active.name) == Some(name.clone());
    let label: SharedString = name.clone().into();

    let mut row = div().flex().items_center().gap_2().child(choice_pill(
        ElementId::Name(format!("app-settings-size-preset-{name}").into()),
        label,
        lit,
        {
            let preset = preset.clone();
            cx.listener(move |this, _, window, cx| this.apply_size_preset(&preset, window, cx))
        },
    ));

    // Only a preset the user made — or one they have saved over — can be renamed or deleted. A
    // built-in is code, so there is nothing of theirs to act on.
    if saved {
        let rename = name.clone();
        let delete = name.clone();
        row = row
            .child(ghost_button(
                ElementId::Name(format!("app-settings-size-rename-{name}").into()),
                None,
                "Rename",
                cx.listener(move |this, _, window, cx| {
                    this.ask_size_preset_rename(rename.clone(), window, cx)
                }),
            ))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-size-delete-{name}").into()),
                None,
                "Delete",
                cx.listener(move |this, _, _, cx| this.delete_size_preset(&delete, cx)),
            ));
    }

    row.into_any_element()
}

/// Every preset, one row each, for the settings section's list.
pub fn preset_list(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let rows: Vec<AnyElement> = app
        .size_presets()
        .iter()
        .map(|preset| preset_row(app, preset, cx))
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(rows)
        .into_any_element()
}

/// A slider in a settings row, at the width every control on that page is given.
pub fn settings_slider(control: AnyElement) -> AnyElement {
    div()
        .w(px(theme::scaled(220.)))
        .flex_none()
        .child(control)
        .into_any_element()
}
