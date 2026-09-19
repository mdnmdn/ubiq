//! Author-made themes as the window acts on them: the fork, the editor, and the name prompt.
//!
//! A custom theme is a fork of a built-in palette plus a sparse override map (`D152`). The
//! resolution lives in `theme.rs`, which is the only file that knows what a token is; what is here
//! is everything *around* the map — which theme is worn, which of its tokens the picker is pointed
//! at, and the three gestures that make, name and unmake one.
//!
//! **Editing a theme wears it.** There is no draft and no OK button: the editor writes through to
//! `workbench.custom_themes`, pushes the list at `theme::set_custom_themes`, and re-resolves the
//! window. That is what makes the specimen strip a specimen of the window — it is
//! `ui/sink/style.rs`'s own `tokens()`, reading the same accessors every screen reads — rather
//! than a second renderer's idea of one.

use gpui::{Context, Focusable as _, Window};

use super::AppState;
use crate::state::{ThemeEditor, ThemePrompt};
use crate::theme::{self, CustomTheme, ThemeId};

impl AppState {
    /// The themes the user authored, in the order they were made.
    pub fn custom_themes(&self) -> &[CustomTheme] {
        &self.workbench.custom_themes
    }

    /// One of them, by id.
    pub fn custom_theme(&self, id: ThemeId) -> Option<&CustomTheme> {
        self.workbench
            .custom_themes
            .iter()
            .find(|theme| theme.id == id.slug())
    }

    /// Hand the list to `theme.rs`, re-resolve the palette in force, and write it all down.
    ///
    /// Every edit ends here: the thread-local registry is what `resolve` reads, so a map that
    /// changed without this is a theme the window cannot see.
    fn settle_custom_themes(&mut self, cx: &mut Context<Self>) {
        theme::set_custom_themes(self.workbench.custom_themes.clone());
        theme::set_theme(self.workbench.theme_id, theme::accent_id(), cx);
        self.remember_interface();
        self.redress_terminals(cx);
        cx.notify();
    }

    /// Put the custom themes the host had stored in force, and fall back when the palette they
    /// were carrying no longer names one.
    ///
    /// A blob can name a theme this config root does not hold — it was deleted by another window,
    /// or the blob came from elsewhere — and a dangling slug would paint the default while still
    /// being written back. Resolving it to the built-in is the same posture `ThemeId::from_slug`
    /// takes for an unknown palette.
    pub(super) fn adopt_custom_themes(&mut self, themes: Vec<CustomTheme>) {
        self.workbench.custom_themes = themes;
        theme::set_custom_themes(self.workbench.custom_themes.clone());
        let id = self.workbench.theme_id;
        if id.is_custom() && !id.is_known_custom() {
            self.workbench.theme_id = id.base();
        }
    }

    // ── The editor ──────────────────────────────────────────────────

    /// Open the editor on a theme, wearing it.
    ///
    /// Editing a palette the window is not drawn in would leave the author picking colours against
    /// somebody else's ground, so the gesture that opens the editor is also the gesture that puts
    /// the theme on.
    pub fn edit_custom_theme(&mut self, id: ThemeId, window: &mut Window, cx: &mut Context<Self>) {
        if self.custom_theme(id).is_none() {
            return;
        }
        if self.workbench.theme_id != id {
            self.set_palette(id, cx);
        }
        let token = theme::EDITABLE_TOKENS[0].key;
        self.workbench.theme_editor = Some(ThemeEditor { theme: id, token });
        self.sync_theme_hex(window, cx);
        cx.notify();
    }

    pub fn close_theme_editor(&mut self, cx: &mut Context<Self>) {
        self.workbench.theme_editor = None;
        cx.notify();
    }

    /// Point the picker at another token. The hex field follows, because it prints whatever the
    /// picker is on.
    pub fn select_theme_token(
        &mut self,
        token: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.workbench.theme_editor.as_mut() {
            editor.token = token;
        }
        self.sync_theme_hex(window, cx);
        cx.notify();
    }

    /// What the editor is pointed at, and what the palette in force draws it at.
    pub fn theme_token_colour(&self) -> gpui::Rgba {
        let palette = theme::Theme::current().palette;
        match &self.workbench.theme_editor {
            Some(editor) => theme::token_colour(&palette, editor.token),
            None => palette.text.primary,
        }
    }

    /// Whether the token in hand is one the author wrote, rather than one inherited from the fork.
    pub fn theme_token_is_written(&self) -> bool {
        self.workbench
            .theme_editor
            .as_ref()
            .and_then(|editor| self.custom_theme(editor.theme).map(|t| (t, editor.token)))
            .is_some_and(|(theme, token)| theme.overrides.contains_key(token))
    }

    /// Write the token the picker is on.
    pub fn set_theme_token_hsv(
        &mut self,
        hue: f32,
        sat: f32,
        val: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.write_theme_token(theme::hsv_to_rgb(hue, sat, val), cx);
        self.sync_theme_hex(window, cx);
    }

    /// The same from the hex field, which is the picker's other half.
    pub(super) fn apply_theme_hex(&mut self, cx: &mut Context<Self>) {
        let text = self.theme_hex_input.read(cx).value();
        let Some(rgb) = crate::state::sink::parse_hex(text.as_ref()) else {
            return;
        };
        if rgb != self.written_theme_token() {
            self.write_theme_token(rgb, cx);
        }
    }

    /// Give the token back to the fork. An override map is sparse, so *inherit* is the absence of
    /// a key rather than a key holding the base's colour — which is what lets a retuned built-in
    /// still reach a theme forked from it.
    pub fn clear_theme_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.workbench.theme_editor.clone() else {
            return;
        };
        if let Some(theme) = self
            .workbench
            .custom_themes
            .iter_mut()
            .find(|theme| theme.id == editor.theme.slug())
        {
            theme.overrides.remove(editor.token);
        }
        self.settle_custom_themes(cx);
        self.sync_theme_hex(window, cx);
    }

    fn write_theme_token(&mut self, rgb: u32, cx: &mut Context<Self>) {
        let Some(editor) = self.workbench.theme_editor.clone() else {
            return;
        };
        if let Some(theme) = self
            .workbench
            .custom_themes
            .iter_mut()
            .find(|theme| theme.id == editor.theme.slug())
        {
            theme.overrides.insert(editor.token.to_string(), rgb);
        }
        self.settle_custom_themes(cx);
    }

    /// The token as the map holds it, or as the palette draws it when nothing was written.
    fn written_theme_token(&self) -> u32 {
        let colour = self.theme_token_colour();
        crate::state::sink::rgb_from_channels(colour.r, colour.g, colour.b)
    }

    /// Print the token in hand into the hex field.
    fn sync_theme_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = crate::state::sink::hex_string(self.written_theme_token());
        let input = self.theme_hex_input.clone();
        input.update(cx, |input, cx| input.set_value(&hex, window, cx));
    }

    // ── Making, naming and unmaking one ─────────────────────────────

    /// **New theme…** — the prompt over the palette in force, which is what the new one forks.
    pub fn ask_new_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.workbench.theme_id.base();
        self.workbench.theme_prompt = Some(ThemePrompt::New { base });
        self.seed_theme_name("", window, cx);
    }

    /// The same prompt over a theme that exists, seeded with what it is called now.
    pub fn ask_rename_theme(&mut self, id: ThemeId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(theme) = self.custom_theme(id) else {
            return;
        };
        let name = theme.name.clone();
        self.workbench.theme_prompt = Some(ThemePrompt::Rename { theme: id });
        self.seed_theme_name(&name, window, cx);
    }

    fn seed_theme_name(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.theme_name_input.clone();
        input.update(cx, |state, cx| state.set_value(value, window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub fn close_theme_prompt(&mut self, cx: &mut Context<Self>) {
        self.workbench.theme_prompt = None;
        cx.notify();
    }

    /// Answer it. A new theme is made empty — every token inherited — worn, and opened in the
    /// editor, which is the only sequence that makes sense: an author asked for a theme in order
    /// to change something about it.
    pub fn confirm_theme_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.workbench.theme_prompt.take() else {
            return;
        };
        let name = self.theme_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }

        match prompt {
            ThemePrompt::New { base } => {
                let theme = CustomTheme {
                    id: theme::new_custom_id(),
                    name,
                    base,
                    overrides: Default::default(),
                };
                let id = theme.theme_id();
                self.workbench.custom_themes.push(theme);
                theme::set_custom_themes(self.workbench.custom_themes.clone());
                self.set_palette(id, cx);
                self.edit_custom_theme(id, window, cx);
            }
            ThemePrompt::Rename { theme } => {
                if let Some(held) = self
                    .workbench
                    .custom_themes
                    .iter_mut()
                    .find(|held| held.id == theme.slug())
                {
                    held.name = name;
                }
                self.settle_custom_themes(cx);
            }
        }
        cx.notify();
    }

    /// Forget a theme.
    ///
    /// **A theme deleted while it is worn falls back to its base**, never to a dangling slug: the
    /// window is drawn from whatever `theme_id` says, so leaving it naming something that no
    /// longer exists is a window painted in the default while still writing the dead name down.
    pub fn delete_custom_theme(&mut self, id: ThemeId, cx: &mut Context<Self>) {
        let base = id.base();
        self.workbench
            .custom_themes
            .retain(|theme| theme.id != id.slug());
        if self
            .workbench
            .theme_editor
            .as_ref()
            .is_some_and(|e| e.theme == id)
        {
            self.workbench.theme_editor = None;
        }
        theme::set_custom_themes(self.workbench.custom_themes.clone());
        if self.workbench.theme_id == id {
            self.set_palette(base, cx);
        } else {
            self.settle_custom_themes(cx);
        }
    }
}

/// Whether a name may be given: non-empty, and — for a rename — different from what it was.
/// Two themes may share a name; they are told apart by their id, and forbidding it would be a rule
/// the author did not ask for.
pub fn theme_name_valid(value: &str, prompt: &ThemePrompt, current: Option<&str>) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    match prompt {
        ThemePrompt::New { .. } => true,
        ThemePrompt::Rename { .. } => Some(value) != current,
    }
}
