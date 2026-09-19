//! Author-made themes: the Appearance row, the editor, and the name prompt.
//!
//! A custom theme is a fork of a built-in palette plus a sparse override map (`D152`), so what is
//! drawn here is a list of overridable tokens and one colour picker — never a full palette. The
//! editable set is **grounds and ink**: `theme::EDITABLE_TOKENS` holds it, and status, ribbon,
//! terminal and project-swatch tokens are deliberately not in it.
//!
//! Three things are borrowed rather than written: the picker is `kit::colour_picker`, the naming
//! question is `kit::prompt_modal`, and the specimen strip is `ui/sink/style.rs`'s own `tokens()`.

use gpui::{
    AnyElement, Context, ElementId, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::{ThemeEditor, ThemePrompt};
use crate::theme::{self, Family, Role, TokenGroup};
use crate::ui::kit::{
    choice_pill, colour_picker, ghost_button, icon_button, modal_sized, primary_button,
    prompt_modal, section_label, state_chip,
};
use crate::ui::{eid, handler, hsv};

/// The Appearance section's **Themes** row: what the user has authored, as pills beside the
/// built-ins above, each with an edit affordance — and **New theme…**, which forks whatever the
/// window is wearing.
pub fn themes_row(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.workbench.theme_id;
    let mut row = div().flex().flex_wrap().items_center().gap_1();

    for custom in app.custom_themes() {
        let id = custom.theme_id();
        let label: SharedString = custom.name.clone().into();
        row = row
            .child(choice_pill(
                eid("app-settings-theme", &custom.id),
                label,
                current == id,
                cx.listener(move |this, _, _, cx| this.set_palette(id, cx)),
            ))
            .child(icon_button(
                eid("app-settings-theme-edit", &custom.id),
                IconName::Settings,
                false,
                cx.listener(move |this, _, window, cx| this.edit_custom_theme(id, window, cx)),
            ));
    }

    row.child(ghost_button(
        "app-settings-theme-new",
        None,
        "New theme\u{2026}",
        cx.listener(|this, _, window, cx| this.ask_new_theme(window, cx)),
    ))
    .into_any_element()
}

/// The editor, while it is up.
///
/// A `kit::modal_sized` rather than more `setting_row`s: Appearance is a fixed 820×560 panel with
/// a 220px nav, and a token list beside a colour picker above a specimen of the whole palette does
/// not fit in what is left of it.
pub fn editor(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(editor) = app.workbench.theme_editor.clone() else {
        return div().into_any_element();
    };
    let id = editor.theme;
    let view = cx.entity();
    let title = format!("Theme \u{2014} {}", id.name());

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_3()
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .gap_3()
                .child(token_list(app, &editor, cx))
                .child(picker(app, &editor, window, cx)),
        )
        .child(specimen())
        .into_any_element();

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "theme-editor-rename",
            None,
            "Rename\u{2026}",
            cx.listener(move |this, _, window, cx| this.ask_rename_theme(id, window, cx)),
        ))
        .child(ghost_button(
            "theme-editor-delete",
            Some(IconName::Delete),
            "Delete",
            cx.listener(move |this, _, _, cx| this.delete_custom_theme(id, cx)),
        ))
        .child(primary_button(
            "theme-editor-done",
            None,
            "Done",
            cx.listener(|this, _, _, cx| this.close_theme_editor(cx)),
        ))
        .into_any_element();

    modal_sized(
        "theme-editor",
        theme::accent(),
        theme::theme_editor_width(),
        Some(theme::theme_editor_height()),
        &title,
        body,
        footer,
        handler(&view, |this, _, cx| this.close_theme_editor(cx)),
        window,
    )
}

/// The editable set down the left, grouped the way `theme.rs` groups it.
fn token_list(app: &AppState, editor: &ThemeEditor, cx: &mut Context<AppState>) -> AnyElement {
    let palette = theme::Theme::current().palette;
    let written = app
        .custom_theme(editor.theme)
        .map(|custom| custom.overrides.clone())
        .unwrap_or_default();

    let mut list = div()
        .id("theme-editor-tokens")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(theme::theme_token_list_width()))
        .gap_1()
        .pr_2()
        .overflow_y_scroll();

    for group in TokenGroup::ALL {
        list = list.child(div().pt_1().child(section_label(group.name())));
        for spec in theme::EDITABLE_TOKENS.iter().filter(|s| s.group == group) {
            let key = spec.key;
            let lit = editor.token == key;
            let colour = theme::token_colour(&palette, key);
            list = list.child(
                div()
                    .id(ElementId::Name(format!("theme-token-{key}").into()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_0p5()
                    .cursor_pointer()
                    .when_lit(lit)
                    .child(
                        div()
                            .size(px(theme::scaled(12.)))
                            .flex_none()
                            .bg(colour)
                            .border_1()
                            .border_color(theme::border()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(theme::font(Family::Chrome, Role::Label))
                            .text_color(theme::text())
                            .child(spec.label),
                    )
                    // A dot marks a token the author wrote: the rest are the fork's, and the
                    // difference is the whole of what a sparse map means.
                    .when_written(written.contains_key(key))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_theme_token(key, window, cx)
                    })),
            );
        }
    }

    list.into_any_element()
}

/// The picker, the contrast warning, and the one control that takes a token back.
fn picker(
    app: &AppState,
    editor: &ThemeEditor,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let current = app.theme_token_colour();
    let rgb = crate::state::sink::rgb_from_channels(current.r, current.g, current.b);
    let (hue, sat, val) = theme::rgb_to_hsv(rgb);
    let focused = app
        .theme_hex_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let token = editor.token;

    let mut column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .gap_2()
        .child(colour_picker(
            "theme-editor",
            hue,
            sat,
            val,
            current,
            &app.theme_hex_input,
            focused,
            hsv(&view, |this, hue, sat, val, window, cx| {
                this.set_theme_token_hsv(hue, sat, val, window, cx)
            }),
        ));

    // The warning, never a block: a theme is taste, and `readable_on` already corrects the one
    // axis where Ubiq decides rather than the author.
    if let Some(warning) = contrast_warning(token) {
        column = column.child(state_chip(
            SharedString::from(warning),
            theme::warning(),
            1.0,
        ));
    }

    if app.theme_token_is_written() {
        column = column.child(div().child(ghost_button(
            "theme-editor-inherit",
            None,
            "Inherit from the base",
            cx.listener(|this, _, window, cx| this.clear_theme_token(window, cx)),
        )));
    }

    column.into_any_element()
}

/// What a token reads at against the surface it is paired with, when that is under WCAG's floor
/// for body text.
fn contrast_warning(token: &str) -> Option<String> {
    let spec = theme::token_spec(token)?;
    let against = spec.against?;
    let palette = theme::Theme::current().palette;
    let ratio = theme::contrast(
        theme::token_colour(&palette, token),
        theme::token_colour(&palette, against),
    );
    (ratio < theme::TEXT_MIN_CONTRAST).then(|| {
        let label = theme::token_spec(against).map_or(against, |spec| spec.label);
        format!("{ratio:.1}:1 against {label} \u{2014} under 4.5:1")
    })
}

/// The live specimen: every token the palette resolves to, drawn by the style reference's own
/// renderer so what an author sees is what the window is actually painting.
fn specimen() -> AnyElement {
    div()
        .id("theme-editor-specimen")
        .flex_none()
        .h(px(theme::theme_specimen_height()))
        .overflow_y_scroll()
        .border_t_1()
        .border_color(theme::border())
        .child(crate::ui::sink::style::tokens())
        .into_any_element()
}

/// The name prompt both **New theme…** and the editor's Rename raise.
pub fn name_prompt(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(prompt) = app.workbench.theme_prompt.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let value = app.theme_name_input.read(cx).value().to_string();
    let current = match &prompt {
        ThemePrompt::Rename { theme } => app.custom_theme(*theme).map(|t| t.name.clone()),
        ThemePrompt::New { .. } => None,
    };
    let enabled = crate::app::theme_name_valid(&value, &prompt, current.as_deref());
    let (title, note, confirm) = match &prompt {
        ThemePrompt::New { base } => (
            format!("New theme from {}", base.name()),
            "A theme is a fork of this palette plus the colours you change. Everything you do \
             not write stays the palette's \u{2014} including status, ribbon and terminal \
             colours, which carry meaning rather than taste."
                .to_string(),
            "Create",
        ),
        ThemePrompt::Rename { .. } => (
            "Rename theme".to_string(),
            "The theme keeps every colour it has; only what it is called changes.".to_string(),
            "Rename",
        ),
    };

    prompt_modal(
        "theme-name",
        &title,
        Some(&note),
        "Name",
        &app.theme_name_input,
        confirm,
        enabled,
        handler(&view, |this, window, cx| {
            this.confirm_theme_prompt(window, cx)
        }),
        handler(&view, |this, _, cx| this.close_theme_prompt(cx)),
        window,
        cx,
    )
}

/// The two states a token row carries, as extensions rather than `when` chains at the call site.
trait TokenRow {
    fn when_lit(self, lit: bool) -> Self;
    fn when_written(self, written: bool) -> Self;
}

impl TokenRow for gpui::Stateful<gpui::Div> {
    fn when_lit(self, lit: bool) -> Self {
        if lit {
            self.bg(theme::selected())
        } else {
            self.hover(|row| row.bg(theme::hover()))
        }
    }

    fn when_written(self, written: bool) -> Self {
        if written {
            self.child(
                div()
                    .size(px(theme::scaled(5.)))
                    .flex_none()
                    .bg(theme::accent()),
            )
        } else {
            self
        }
    }
}
