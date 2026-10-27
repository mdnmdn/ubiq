//! The Tools dialog's mutators: opening it, following a suggestion, every option, and loading a
//! file into a field.
//!
//! **A file is read the way a dropped guest file already is** — [`super::read_guest_file`], on the
//! interface's side, because a path dropped from the platform's file manager or picked in its
//! chooser is by construction a path on the machine the window is drawn on. Nothing new crosses
//! `messages.rs` for it.

use std::path::PathBuf;

use chrono::Utc;
use gpui::{EventEmitter, PathPromptOptions};

use crate::devtools::{codec, detect, diff, hash, jwt, lorem, time, uuids};
use crate::state::devtools::{
    CLIPBOARD_SCAN_MAX, DevtoolsInputs, DevtoolsState, FileTarget, LOREM_MAX, Suggestion,
    ToolSection, UUID_MAX, auto_follows,
};

use super::*;

/// How long the diff waits for typing to pause before it recomputes.
const DIFF_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);

/// Which field changed — finer than a section, because two sections have two fields each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Smart,
    Base64,
    Jwt,
    Time,
    Url,
    Hash,
    HashExpect,
    UuidName,
    UuidInspect,
    Diff,
}

impl AppState {
    /// The titlebar's wrench: open the dialog on Smart, or put it away if it is up.
    pub fn toggle_devtools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.devtools.is_some() {
            self.close_devtools(cx);
        } else {
            self.open_devtools(window, cx);
        }
    }

    pub fn open_devtools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.devtools.is_some() {
            return;
        }
        self.workbench.open_menu = None;

        let area = |window: &mut Window, cx: &mut Context<Self>, hint: &str, rows: usize| {
            let hint = hint.to_string();
            cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(hint)
                    .auto_grow(rows, rows.max(12))
            })
        };
        let line = |window: &mut Window, cx: &mut Context<Self>, hint: &str| {
            let hint = hint.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        };
        let inputs = DevtoolsInputs {
            smart: area(
                window,
                cx,
                "Paste anything — a token, a timestamp, a hash, encoded text — or drop a file",
                6,
            ),
            base64: area(window, cx, "Text to encode, or Base64 to decode", 5),
            jwt: area(window, cx, "Paste a JWT (a Bearer prefix is fine)", 4),
            time: line(
                window,
                cx,
                "1700000000, 1700000000000, 2023-11-14T22:13:20Z, Tue, 14 Nov 2023…",
            ),
            url: area(window, cx, "Text to encode, or %-escaped text to decode", 4),
            hash: area(window, cx, "Text to hash", 4),
            hash_expect: line(window, cx, "Optional: a digest to compare against"),
            uuid_name: line(window, cx, "Name (v5)"),
            uuid_inspect: line(window, cx, "Paste a UUID to inspect it"),
            diff_left: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Original — paste, or drop a file")
            }),
            diff_right: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Changed — paste, or drop a file")
            }),
        };

        let mut subscriptions = Vec::new();
        subscriptions.push(watch(&inputs.smart, Field::Smart, window, cx));
        subscriptions.push(watch(&inputs.base64, Field::Base64, window, cx));
        subscriptions.push(watch(&inputs.jwt, Field::Jwt, window, cx));
        subscriptions.push(watch(&inputs.time, Field::Time, window, cx));
        subscriptions.push(watch(&inputs.url, Field::Url, window, cx));
        subscriptions.push(watch(&inputs.hash, Field::Hash, window, cx));
        subscriptions.push(watch(&inputs.hash_expect, Field::HashExpect, window, cx));
        subscriptions.push(watch(&inputs.uuid_name, Field::UuidName, window, cx));
        subscriptions.push(watch(&inputs.uuid_inspect, Field::UuidInspect, window, cx));
        subscriptions.push(watch(&inputs.diff_left, Field::Diff, window, cx));
        subscriptions.push(watch(&inputs.diff_right, Field::Diff, window, cx));

        let focus = inputs.smart.clone();
        let mut state = DevtoolsState::new(inputs);
        state.subscriptions = subscriptions;
        state.lorem_seed = Utc::now().timestamp_millis() as u64;
        self.workbench.devtools = Some(state);

        self.generate_uuids(cx);
        self.refresh_lorem(cx);
        self.scan_clipboard(cx);
        focus.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub fn close_devtools(&mut self, cx: &mut Context<Self>) {
        self.workbench.devtools = None;
        cx.notify();
    }

    pub fn set_devtools_section(&mut self, section: ToolSection, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.section = section;
            cx.notify();
        }
    }

    /// Open the section a suggestion names, with its text in that section's field.
    pub fn follow_suggestion(
        &mut self,
        suggestion: Suggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tools) = self.workbench.devtools.as_mut() else {
            return;
        };
        tools.section = suggestion.section;
        if tools.clipboard.as_ref() == Some(&suggestion) {
            tools.clipboard = None;
        }
        let text = suggestion.text.clone();
        let field = match suggestion.section {
            ToolSection::Base64 => {
                tools.b64_decode = suggestion.decode;
                Some((Field::Base64, Target::Area(tools.inputs.base64.clone())))
            }
            ToolSection::Jwt => Some((Field::Jwt, Target::Area(tools.inputs.jwt.clone()))),
            ToolSection::Time => Some((Field::Time, Target::Line(tools.inputs.time.clone()))),
            ToolSection::Url => {
                tools.url_decode = suggestion.decode;
                Some((Field::Url, Target::Area(tools.inputs.url.clone())))
            }
            ToolSection::Hash => Some((
                Field::HashExpect,
                Target::Line(tools.inputs.hash_expect.clone()),
            )),
            ToolSection::Uuid => Some((
                Field::UuidInspect,
                Target::Line(tools.inputs.uuid_inspect.clone()),
            )),
            ToolSection::Smart | ToolSection::Lorem | ToolSection::Diff => None,
        };
        if let Some((field, target)) = field {
            target.set(text, window, cx);
            self.devtools_changed(field, false, window, cx);
        }
        cx.notify();
    }

    fn devtools_changed(
        &mut self,
        field: Field,
        pasted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tools) = self.workbench.devtools.as_mut() else {
            return;
        };
        let value = |entity: &Entity<TextareaState>, cx: &App| entity.read(cx).value().to_string();
        let line = |entity: &Entity<InputState>, cx: &App| entity.read(cx).value().to_string();
        match field {
            Field::Smart => {
                let text = value(&tools.inputs.smart, cx);
                // A jump of more than one character at once is a paste or a drop. Only that
                // follows a lone match by itself: typing never moves the dialog. Counted in
                // characters, not bytes, so a typed é or emoji is one keystroke.
                let chars = text.chars().count();
                let pasted = pasted || chars > tools.smart_len + 1;
                tools.smart_len = chars;
                tools.smart_found = detect::detect(&text);
                if pasted
                    && let [kind] = tools.smart_found[..]
                    && auto_follows(kind)
                {
                    let suggestion = Suggestion::new(kind, &text);
                    self.follow_suggestion(suggestion, window, cx);
                    return;
                }
            }
            Field::Base64 => {
                tools.b64_out = b64(tools, &value(&tools.inputs.base64, cx));
            }
            Field::Jwt => {
                let text = value(&tools.inputs.jwt, cx);
                tools.jwt_out = (!text.trim().is_empty()).then(|| jwt::decode(&text, Utc::now()));
            }
            Field::Time => {
                let text = line(&tools.inputs.time, cx);
                tools.time_out = (!text.trim().is_empty()).then(|| {
                    time::parse(&text, &chrono::Local)
                        .map(|(at, read)| (read, time::rows(at, &chrono::Local, Utc::now())))
                });
            }
            Field::Url => {
                tools.url_out = url(tools, &value(&tools.inputs.url, cx));
            }
            Field::Hash => {
                let text = value(&tools.inputs.hash, cx);
                tools.hash_out = hash::digests(text.as_bytes());
            }
            // Compared at paint: nothing to compute.
            Field::HashExpect => {}
            Field::UuidName => {
                if tools.uuid_version == uuids::UuidVersion::V5 {
                    self.generate_uuids(cx);
                }
            }
            Field::UuidInspect => {
                let text = line(&tools.inputs.uuid_inspect, cx);
                tools.uuid_inspect_out = (!text.trim().is_empty()).then(|| uuids::inspect(&text));
            }
            Field::Diff => {
                let left = value(&tools.inputs.diff_left, cx);
                let right = value(&tools.inputs.diff_right, cx);
                // Debounced and off the UI thread, like the outline: a keystroke in a large pane
                // never waits on the diff, and only the latest request's answer lands.
                tools.diff_gen = tools.diff_gen.wrapping_add(1);
                let token = tools.diff_gen;
                if left.is_empty() && right.is_empty() {
                    tools.diff = None;
                } else {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(DIFF_DEBOUNCE).await;
                        let result = cx
                            .background_spawn(async move { diff::line_diff(&left, &right) })
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if let Some(tools) = this.workbench.devtools.as_mut()
                                && tools.diff_gen == token
                            {
                                tools.diff = Some(result);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }

    pub fn set_b64_mode(
        &mut self,
        decode: bool,
        alphabet: codec::B64Alphabet,
        cx: &mut Context<Self>,
    ) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.b64_decode = decode;
            tools.b64_alphabet = alphabet;
            let text = tools.inputs.base64.read(cx).value().to_string();
            tools.b64_out = b64(tools, &text);
            cx.notify();
        }
    }

    pub fn set_url_mode(&mut self, decode: bool, full: bool, plus: bool, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.url_decode = decode;
            tools.url_full = full;
            tools.url_plus = plus;
            let text = tools.inputs.url.read(cx).value().to_string();
            tools.url_out = url(tools, &text);
            cx.notify();
        }
    }

    /// Put the output back into the input and flip the direction — encode, then decode what came
    /// out, without copying anything.
    pub fn swap_devtools_output(
        &mut self,
        section: ToolSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tools) = self.workbench.devtools.as_mut() else {
            return;
        };
        let (out, target, field) = match section {
            ToolSection::Base64 => {
                let Ok(out) = tools.b64_out.clone() else {
                    return;
                };
                tools.b64_decode = !tools.b64_decode;
                (out, tools.inputs.base64.clone(), Field::Base64)
            }
            ToolSection::Url => {
                let Ok(out) = tools.url_out.clone() else {
                    return;
                };
                tools.url_decode = !tools.url_decode;
                (out, tools.inputs.url.clone(), Field::Url)
            }
            _ => return,
        };
        target.update(cx, |state, cx| state.set_value(out, window, cx));
        self.devtools_changed(field, false, window, cx);
    }

    /// The time section's "Now".
    pub fn devtools_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tools) = self.workbench.devtools.as_ref() else {
            return;
        };
        let field = tools.inputs.time.clone();
        let now = Utc::now().timestamp().to_string();
        field.update(cx, |state, cx| state.set_value(now, window, cx));
        self.devtools_changed(Field::Time, false, window, cx);
    }

    pub fn set_uuid_options(
        &mut self,
        version: uuids::UuidVersion,
        namespace: uuids::Namespace,
        format: uuids::UuidFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(tools) = self.workbench.devtools.as_mut() else {
            return;
        };
        let regenerate = tools.uuid_version != version || tools.uuid_namespace != namespace;
        tools.uuid_version = version;
        tools.uuid_namespace = namespace;
        tools.uuid_format = format;
        if regenerate {
            self.generate_uuids(cx);
        }
        cx.notify();
    }

    pub fn nudge_uuid_count(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.uuid_count = tools
                .uuid_count
                .saturating_add_signed(delta)
                .clamp(1, UUID_MAX);
            self.generate_uuids(cx);
        }
    }

    pub fn generate_uuids(&mut self, cx: &mut Context<Self>) {
        let Some(tools) = self.workbench.devtools.as_mut() else {
            return;
        };
        let name = tools.inputs.uuid_name.read(cx).value().to_string();
        tools.uuid_out = uuids::generate(
            tools.uuid_version,
            tools.uuid_count,
            tools.uuid_namespace,
            &name,
        );
        cx.notify();
    }

    pub fn set_lorem_unit(&mut self, unit: lorem::LoremUnit, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.lorem_unit = unit;
            self.refresh_lorem(cx);
        }
    }

    pub fn nudge_lorem_count(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.lorem_count = tools
                .lorem_count
                .saturating_add_signed(delta)
                .clamp(1, LOREM_MAX);
            self.refresh_lorem(cx);
        }
    }

    /// Another draw of the same shape.
    pub fn reseed_lorem(&mut self, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.lorem_seed = tools.lorem_seed.wrapping_add(1);
            self.refresh_lorem(cx);
        }
    }

    fn refresh_lorem(&mut self, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.lorem_out =
                lorem::generate(tools.lorem_unit, tools.lorem_count, tools.lorem_seed);
            cx.notify();
        }
    }

    pub fn set_diff_split(&mut self, split: bool, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.diff_split = split;
            cx.notify();
        }
    }

    /// Exchange the two diff panes.
    pub fn swap_diff_sides(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tools) = self.workbench.devtools.as_ref() else {
            return;
        };
        let (left, right) = (
            tools.inputs.diff_left.clone(),
            tools.inputs.diff_right.clone(),
        );
        let a = left.read(cx).value().to_string();
        let b = right.read(cx).value().to_string();
        left.update(cx, |state, cx| state.set_value(b, window, cx));
        right.update(cx, |state, cx| state.set_value(a, window, cx));
        self.devtools_changed(Field::Diff, false, window, cx);
    }

    /// The "Analyse clipboard automatically" box. Persisted with the interface's settings; turning
    /// it on looks at the clipboard at once rather than at the next open.
    pub fn toggle_devtools_clipboard(&mut self, cx: &mut Context<Self>) {
        let on = !self.workbench.settings.ui.devtools_clipboard;
        self.workbench.settings.ui.devtools_clipboard = on;
        self.remember_settings();
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.clipboard = None;
        }
        self.scan_clipboard(cx);
        cx.notify();
    }

    /// Look at the clipboard and keep an offer if its text is something a section reads.
    fn scan_clipboard(&mut self, cx: &mut Context<Self>) {
        if !self.workbench.settings.ui.devtools_clipboard {
            return;
        }
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .filter(|text| !text.trim().is_empty() && text.len() <= CLIPBOARD_SCAN_MAX);
        let Some(text) = text else {
            return;
        };
        let offer = detect::detect(&text)
            .first()
            .map(|kind| Suggestion::new(*kind, &text));
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.clipboard = offer;
        }
    }

    pub fn dismiss_devtools_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(tools) = self.workbench.devtools.as_mut() {
            tools.clipboard = None;
            cx.notify();
        }
    }

    /// Ask the platform's chooser for one file and load it into `target`.
    pub fn pick_devtools_file(
        &mut self,
        target: FileTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Load".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.drop_devtools_paths(target, &paths, window, cx);
            });
        })
        .detach();
    }

    /// A file dropped on a field: the first one is loaded into it.
    pub fn drop_devtools_paths(
        &mut self,
        target: FileTarget,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = paths.first() {
            self.load_devtools_file(target, path, window, cx);
        }
    }

    fn load_devtools_file(
        &mut self,
        target: FileTarget,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tools) = self.workbench.devtools.as_mut() else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let contents = match read_guest_file(path) {
            Ok(contents) => contents,
            Err(reason) => {
                tools.note = Some(format!("Could not read {name}: {reason}"));
                cx.notify();
                return;
            }
        };
        if contents.is_binary {
            tools.note = Some(format!("{name} is not text — nothing loaded"));
            cx.notify();
            return;
        }
        tools.note = contents.truncated.then(|| {
            format!(
                "Loaded the first {} KB of {name} — the file is longer",
                contents.bytes.len() / 1024
            )
        });
        let text = String::from_utf8_lossy(&contents.bytes).into_owned();
        let (entity, field) = match target {
            FileTarget::Smart => (tools.inputs.smart.clone(), Field::Smart),
            FileTarget::DiffLeft => (tools.inputs.diff_left.clone(), Field::Diff),
            FileTarget::DiffRight => (tools.inputs.diff_right.clone(), Field::Diff),
        };
        entity.update(cx, |state, cx| state.set_value(text, window, cx));
        self.devtools_changed(field, true, window, cx);
    }
}

/// A field a suggestion writes into: the multi-line kind or the one-line kind.
enum Target {
    Area(Entity<TextareaState>),
    Line(Entity<InputState>),
}

impl Target {
    fn set(self, text: String, window: &mut Window, cx: &mut Context<AppState>) {
        match self {
            Target::Area(entity) => {
                entity.update(cx, |state, cx| state.set_value(text, window, cx))
            }
            Target::Line(entity) => {
                entity.update(cx, |state, cx| state.set_value(text, window, cx))
            }
        }
    }
}

/// Recompute on every change of one field. `set_value` emits nothing, so a programmatic write
/// calls `devtools_changed` itself.
fn watch<T>(
    entity: &Entity<T>,
    field: Field,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> Subscription
where
    T: EventEmitter<InputEvent> + 'static,
{
    cx.subscribe_in(
        entity,
        window,
        move |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.devtools_changed(field, false, window, cx);
            }
        },
    )
}

fn b64(tools: &DevtoolsState, text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    if tools.b64_decode {
        codec::b64_decode_text(text)
    } else {
        Ok(codec::b64_encode(text.as_bytes(), tools.b64_alphabet))
    }
}

fn url(tools: &DevtoolsState, text: &str) -> Result<String, String> {
    if tools.url_decode {
        codec::url_decode(text, tools.url_plus)
    } else {
        Ok(codec::url_encode(text, tools.url_full))
    }
}
