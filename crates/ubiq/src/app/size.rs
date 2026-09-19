//! The two size axes as the window acts on them: the popover, the presets, and the name prompt.
//!
//! The axes themselves live in `theme.rs` and are moved by `AppState::set_ui_scale` /
//! `set_text_ratio` in `app/shell.rs`, which route through `settle_metrics` so a drag costs one
//! terminal re-dress and one blob write rather than one per frame. What is here is everything
//! *around* those two numbers: which preset they currently spell, the saved list, and the naming
//! question both the popover and the Size settings section raise.

use gpui::{Context, Focusable as _, Window};

use super::AppState;
use crate::state::prefs::{SizePreset, all_size_presets};
use crate::state::{MenuId, SizePrompt};
use crate::theme;

/// Two axis values are the same preset when both land within this of it. A slider stop is `0.05`
/// apart, so half a stop is comfortably inside the nearest one and outside its neighbours.
const PRESET_EPSILON: f32 = 0.005;

impl AppState {
    /// Every preset on offer — the built-ins, then what the user saved.
    pub fn size_presets(&self) -> Vec<SizePreset> {
        all_size_presets(&self.workbench.size_presets)
    }

    /// The preset the axes currently spell, if they spell one. `None` is *Custom*, which is what
    /// the popover's trigger says and why no pill is lit.
    pub fn active_size_preset(&self) -> Option<SizePreset> {
        let metrics = theme::metrics();
        self.size_presets().into_iter().find(|preset| {
            (preset.ui_scale - metrics.ui_scale).abs() < PRESET_EPSILON
                && (preset.text_ratio - metrics.text_ratio).abs() < PRESET_EPSILON
        })
    }

    /// Put the sliders back on the axes.
    ///
    /// The two `SliderState`s are built when the window is, and the stored metrics arrive from
    /// the host afterwards — so rather than a window handle the message path does not have, every
    /// gesture that *opens* a surface drawing them calls this first. A preset, a reset and the
    /// settings nav do the same, because none of those moves the slider the user dragged.
    pub fn sync_size_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let metrics = theme::metrics();
        self.ui_scale_slider.update(cx, |state, cx| {
            state.set_value(metrics.ui_scale, window, cx)
        });
        self.text_ratio_slider.update(cx, |state, cx| {
            state.set_value(metrics.text_ratio, window, cx)
        });
    }

    /// Open the status bar's size popover. A trigger *opens*, never toggles — the one menu rule.
    pub fn open_size_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_size_sliders(window, cx);
        self.open_menu(MenuId::Size, cx);
    }

    /// Move both axes at once, to the point a preset names.
    pub fn apply_size_preset(
        &mut self,
        preset: &SizePreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_ui_scale(preset.ui_scale, cx);
        self.set_text_ratio(preset.text_ratio, cx);
        self.sync_size_sliders(window, cx);
    }

    /// Both axes back to `1.0` — the window every constant declares. The per-family trims are
    /// deliberately left alone: they are a different preference, and Reset here means "undo the
    /// two sliders above me".
    pub fn reset_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_ui_scale(1.0, cx);
        self.set_text_ratio(1.0, cx);
        self.sync_size_sliders(window, cx);
    }

    /// Raise the name prompt for a preset over the axes as they stand.
    pub fn ask_size_preset_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.size_prompt = Some(SizePrompt::Save);
        self.seed_size_name("", window, cx);
    }

    /// Raise the same prompt over a saved preset, seeded with the name it has now.
    pub fn ask_size_preset_rename(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.size_prompt = Some(SizePrompt::Rename { name: name.clone() });
        self.seed_size_name(&name, window, cx);
    }

    fn seed_size_name(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.size_name_input.clone();
        input.update(cx, |state, cx| state.set_value(value, window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub fn close_size_prompt(&mut self, cx: &mut Context<Self>) {
        self.workbench.size_prompt = None;
        cx.notify();
    }

    /// Answer the prompt. Saving under a name already in use **replaces** that preset rather than
    /// adding a second one under it — two rows with one name is a list the user cannot act on.
    pub fn confirm_size_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.workbench.size_prompt.take() else {
            return;
        };
        let name = self.size_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }

        match prompt {
            SizePrompt::Save => {
                let metrics = theme::metrics();
                self.write_size_preset(SizePreset {
                    name,
                    ui_scale: metrics.ui_scale,
                    text_ratio: metrics.text_ratio,
                });
            }
            SizePrompt::Rename { name: old } => {
                // A rename is a delete and a save: the preset keeps its two numbers and takes the
                // new name's place in the list, and the old name stops existing.
                let Some(preset) = self
                    .size_presets()
                    .into_iter()
                    .find(|preset| preset.name == old)
                else {
                    cx.notify();
                    return;
                };
                self.workbench
                    .size_presets
                    .retain(|saved| saved.name != old);
                self.write_size_preset(SizePreset { name, ..preset });
            }
        }
        self.remember_interface();
        cx.notify();
    }

    /// Put a preset in the saved list, replacing any entry of the same name in place.
    fn write_size_preset(&mut self, preset: SizePreset) {
        match self
            .workbench
            .size_presets
            .iter_mut()
            .find(|saved| saved.name.eq_ignore_ascii_case(&preset.name))
        {
            Some(saved) => *saved = preset,
            None => self.workbench.size_presets.push(preset),
        }
    }

    /// Forget a saved preset. A built-in is code and cannot be deleted — what a saved entry
    /// shadowing one loses is the shadow, so the built-in's own numbers come back.
    pub fn delete_size_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        self.workbench
            .size_presets
            .retain(|saved| saved.name != name);
        self.remember_interface();
        cx.notify();
    }

    /// Whether a preset's row offers Rename and Delete: a built-in nobody has overridden does not.
    pub fn size_preset_is_saved(&self, name: &str) -> bool {
        self.workbench
            .size_presets
            .iter()
            .any(|saved| saved.name == name)
    }
}

/// Whether a name may be saved: non-empty, and — for a rename — different from what it was.
///
/// A built-in's name is allowed: saving under it replaces that built-in for this user, which is
/// how a preset the build ships is retuned without a second row appearing under the same word.
pub fn size_name_valid(value: &str, prompt: &SizePrompt) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    match prompt {
        SizePrompt::Save => true,
        SizePrompt::Rename { name } => value != name,
    }
}
