//! The Tools dialog, over the window: a nav of nine sections and the section's page beside it.
//!
//! **The settings page's frame, not a copy of it.** The panel is `SETTINGS_WIDTH`×`SETTINGS_HEIGHT`
//! and the split is `kit::settings_split`, so switching sections never resizes it and the nav reads
//! like the one users already know. What each section computes is `crate::devtools`; this draws
//! the fields, the options and the answers, and every answer carries a copy button.

use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, ElementId, Entity, ExternalPaths, Focusable,
    FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::devtools::codec::B64Alphabet;
use crate::devtools::jwt::ClaimState;
use crate::devtools::lorem::LoremUnit;
use crate::devtools::uuids::{self, Namespace, UuidFormat, UuidVersion};
use crate::devtools::{diff, hash};
use crate::state::Layer;
use crate::state::devtools::{DevtoolsState, FileTarget, Suggestion, ToolSection};
use crate::state::editor::ViewLayout;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    UbiqIcon, check_box, choice_pill, column, field, ghost_button, heading, icon_button, mono,
    nav_item, primary_button, section_label, settings_split, status_dot, stepper, tag,
};

pub fn overlay(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(tools) = app.workbench.devtools.as_ref() else {
        return div().into_any_element();
    };
    let viewport = window.viewport_size();
    let panel = div()
        .id("devtools-dialog")
        .w(px(theme::settings_width()))
        .h(px(theme::settings_height()))
        .max_w(viewport.width)
        .max_h(viewport.height)
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        .child(header(cx))
        .child(settings_split(
            "devtools",
            nav(tools, cx),
            body(app, tools, window, cx),
        ))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if this.covered(Layer::Devtools) {
                return;
            }
            this.close_devtools(cx);
        }));

    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id("devtools")
                .w(viewport.width)
                .h(viewport.height)
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::scrim())
                .occlude()
                .child(panel),
        ),
    )
    .priority(2)
    .into_any_element()
}

fn header(cx: &mut Context<AppState>) -> AnyElement {
    div()
        .h(px(52.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Title))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child("Tools"),
        )
        .child(
            icon_button(
                "devtools-close",
                IconName::Close,
                false,
                cx.listener(|this, _, _, cx| this.close_devtools(cx)),
            )
            .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("Close").build(window, cx)),
        )
        .into_any_element()
}

fn section_icon(section: ToolSection) -> Icon {
    match section {
        ToolSection::Smart => Icon::new(IconName::Asterisk),
        ToolSection::Base64 => Icon::new(IconName::Replace),
        ToolSection::Jwt => Icon::new(IconName::CircleUser),
        ToolSection::Time => Icon::new(IconName::Calendar),
        ToolSection::Url => Icon::new(IconName::Globe),
        ToolSection::Hash => Icon::new(IconName::Cpu),
        ToolSection::Uuid => Icon::new(IconName::Inspector),
        ToolSection::Lorem => Icon::new(IconName::FileText),
        ToolSection::Diff => Icon::new(UbiqIcon::GitModified),
    }
}

fn nav(tools: &DevtoolsState, cx: &mut Context<AppState>) -> Vec<AnyElement> {
    ToolSection::ALL
        .iter()
        .map(|&section| {
            nav_item(
                ElementId::Name(format!("devtools-nav-{}", section.label()).into()),
                section_icon(section),
                section.label(),
                None,
                section == tools.section,
                true,
                cx.listener(move |this, _, _, cx| this.set_devtools_section(section, cx)),
            )
        })
        .collect()
}

fn body(
    app: &AppState,
    tools: &DevtoolsState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    match tools.section {
        ToolSection::Smart => smart(app, tools, window, cx),
        ToolSection::Base64 => base64(tools, window, cx),
        ToolSection::Jwt => jwt(tools, window, cx),
        ToolSection::Time => time(tools, window, cx),
        ToolSection::Url => url(tools, window, cx),
        ToolSection::Hash => hashes(tools, window, cx),
        ToolSection::Uuid => uuid(tools, window, cx),
        ToolSection::Lorem => lorem(tools, cx),
        ToolSection::Diff => diff_section(tools, window, cx),
    }
}

// ── Furniture ───────────────────────────────────────────────────────────────

fn copy_button(id: impl Into<ElementId>, text: String, cx: &mut Context<AppState>) -> AnyElement {
    icon_button(
        id,
        IconName::Copy,
        false,
        cx.listener(move |_, _, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        }),
    )
    .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("Copy").build(window, cx))
    .into_any_element()
}

/// A multi-line field, optionally a drop target for a file.
fn area(
    id: &'static str,
    entity: &Entity<TextareaState>,
    drop: Option<FileTarget>,
    height: Option<f32>,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = entity.read(cx).focus_handle(cx).is_focused(window);
    let mut input = Textarea::new(entity)
        .appearance(false)
        .bordered(false)
        .w_full()
        .text_size(theme::font(Family::Content, Role::Label))
        .font_family(theme::MONO_FONT);
    if let Some(height) = height {
        input = input.h(px(height));
    }
    let focus = entity.clone();
    let mut root = field(theme::border(), focused)
        .id(id)
        .w_full()
        .px_2()
        .py_1p5()
        .cursor_text()
        .child(input)
        .on_click(cx.listener(move |_, _, window, cx| {
            focus.update(cx, |state, cx| state.focus(window, cx));
        }));
    if let Some(target) = drop {
        root = root
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.drop_devtools_paths(target, paths.paths(), window, cx);
                cx.stop_propagation();
            }))
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(theme::accent_soft()));
    }
    root.into_any_element()
}

fn line(entity: &Entity<InputState>, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let focused = entity.read(cx).focus_handle(cx).is_focused(window);
    field(theme::border(), focused)
        .flex_1()
        .min_w(px(0.))
        .h(px(28.))
        .px_2()
        .child(Input::new(entity).appearance(false))
        .into_any_element()
}

/// An answer: mono text on a surface, with its copy button.
fn output(id: &str, text: &str, cx: &mut Context<AppState>) -> AnyElement {
    if text.is_empty() {
        return div().into_any_element();
    }
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_1()
        .bg(theme::pane_bg())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::border())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px_2()
                .py_1p5()
                .font_family(theme::MONO_FONT)
                .text_size(theme::font(Family::Content, Role::Label))
                .text_color(theme::text())
                .child(SharedString::from(text.to_string())),
        )
        .child(copy_button(
            ElementId::Name(format!("{id}-copy").into()),
            text.to_string(),
            cx,
        ))
        .into_any_element()
}

/// One labelled value on one line, copyable.
fn value_row(
    id: impl Into<SharedString>,
    label: &str,
    value: &str,
    tone: Option<gpui::Rgba>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id: SharedString = id.into();
    let mut row = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_muted())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .py_1()
                .font_family(theme::MONO_FONT)
                .text_size(theme::font(Family::Content, Role::Label))
                .text_color(tone.unwrap_or(theme::text()))
                .child(SharedString::from(value.to_string())),
        )
        .child(copy_button(
            ElementId::Name(format!("{id}-copy").into()),
            value.to_string(),
            cx,
        ));
    if let Some(tone) = tone {
        row = row
            .border_l(px(theme::accent_edge()))
            .border_color(tone)
            .pl_2();
    }
    row.into_any_element()
}

fn error(text: &str) -> AnyElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Body))
        .text_color(theme::danger())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn muted(text: &str) -> AnyElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_muted())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn row(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .children(children)
        .into_any_element()
}

fn pill(
    id: &str,
    label: &str,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    choice_pill(
        ElementId::Name(id.to_string().into()),
        label.to_string(),
        active,
        on_click,
    )
    .into_any_element()
}

fn chip(id: String, suggestion: Suggestion, cx: &mut Context<AppState>) -> AnyElement {
    let label = suggestion.chip();
    let tip = format!("Open in the {}", suggestion.section.destination());
    tag(
        ElementId::Name(id.into()),
        label,
        tip,
        theme::accent_soft(),
        theme::accent(),
        theme::text(),
        false,
        cx.listener(move |this, _, window, cx| {
            this.follow_suggestion(suggestion.clone(), window, cx)
        }),
    )
    .into_any_element()
}

fn note(tools: &DevtoolsState) -> Option<AnyElement> {
    tools.note.as_ref().map(|note| {
        div()
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .text_color(theme::warning())
            .child(SharedString::from(note.clone()))
            .into_any_element()
    })
}

// ── Sections ────────────────────────────────────────────────────────────────

fn smart(
    app: &AppState,
    tools: &DevtoolsState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let text = tools.inputs.smart.read(cx).value().to_string();
    let mut items = vec![heading(
        "Smart",
        "Paste anything, or drop a file on the field. What it looks like is offered below; a \
         single clear match opens its section on a paste.",
    )];

    if let Some(offer) = tools.clipboard.clone() {
        let words = format!(
            "Clipboard looks like {} — open in the {}",
            offer.kind.label(),
            offer.section.destination()
        );
        items.push(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1p5()
                .bg(theme::accent_soft())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::accent())
                .child(
                    Icon::new(IconName::Info)
                        .with_size(Size::Small)
                        .text_color(theme::accent()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(SharedString::from(words)),
                )
                .child(primary_button(
                    "devtools-clipboard-open",
                    None,
                    "Open",
                    cx.listener(move |this, _, window, cx| {
                        this.follow_suggestion(offer.clone(), window, cx)
                    }),
                ))
                .child(
                    icon_button(
                        "devtools-clipboard-dismiss",
                        IconName::Close,
                        false,
                        cx.listener(|this, _, _, cx| this.dismiss_devtools_clipboard(cx)),
                    )
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Dismiss").build(window, cx)
                    }),
                )
                .into_any_element(),
        );
    }

    items.push(area(
        "devtools-smart",
        &tools.inputs.smart,
        Some(FileTarget::Smart),
        None,
        window,
        cx,
    ));
    items.push(row(vec![
        ghost_button(
            "devtools-smart-load",
            Some(IconName::FolderOpen),
            "Load file…",
            cx.listener(|this, _, window, cx| {
                this.pick_devtools_file(FileTarget::Smart, window, cx)
            }),
        )
        .into_any_element(),
    ]));
    items.extend(note(tools));

    if !tools.smart_found.is_empty() {
        let mut chips = vec![muted(&format!(
            "Looks like {} —",
            tools.smart_found[0].label()
        ))];
        for (i, offer) in tools.smart_suggestions(&text).into_iter().enumerate() {
            chips.push(chip(format!("devtools-smart-chip-{i}"), offer, cx));
        }
        items.push(row(chips));
    } else if !text.trim().is_empty() {
        items.push(muted(
            "Nothing recognised — pick a section on the left to work on it.",
        ));
    }

    let on = app.workbench.settings.ui.devtools_clipboard;
    items.push(
        div()
            .pt_2()
            .flex()
            .items_center()
            .gap_2()
            .child(check_box(
                "devtools-clipboard-auto",
                on,
                cx.listener(|this, _, _, cx| this.toggle_devtools_clipboard(cx)),
            ))
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(theme::text())
                    .child("Analyse clipboard automatically"),
            )
            .child(muted("— when the dialog opens"))
            .into_any_element(),
    );
    column(items)
}

fn base64(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let decode = tools.b64_decode;
    let alphabet = tools.b64_alphabet;
    let mut items = vec![
        heading(
            "Base64",
            "Encode text, or decode Base64 back to it. Decoding takes either alphabet, padded or \
             not, and ignores line breaks.",
        ),
        row(vec![
            pill(
                "devtools-b64-encode",
                "Encode",
                !decode,
                cx.listener(move |this, _, _, cx| this.set_b64_mode(false, alphabet, cx)),
            ),
            pill(
                "devtools-b64-decode",
                "Decode",
                decode,
                cx.listener(move |this, _, _, cx| this.set_b64_mode(true, alphabet, cx)),
            ),
            div().w(px(12.)).into_any_element(),
            pill(
                "devtools-b64-standard",
                "Standard",
                alphabet == B64Alphabet::Standard,
                cx.listener(move |this, _, _, cx| {
                    this.set_b64_mode(decode, B64Alphabet::Standard, cx)
                }),
            ),
            pill(
                "devtools-b64-urlsafe",
                "URL-safe",
                alphabet == B64Alphabet::UrlSafe,
                cx.listener(move |this, _, _, cx| {
                    this.set_b64_mode(decode, B64Alphabet::UrlSafe, cx)
                }),
            ),
        ]),
        area(
            "devtools-b64-in",
            &tools.inputs.base64,
            None,
            None,
            window,
            cx,
        ),
    ];
    match &tools.b64_out {
        Ok(out) if !out.is_empty() => {
            items.push(output("devtools-b64-out", out, cx));
            items.push(swap_button("devtools-b64-swap", ToolSection::Base64, cx));
        }
        Ok(_) => {}
        Err(e) => items.push(error(e)),
    }
    column(items)
}

fn swap_button(id: &'static str, section: ToolSection, cx: &mut Context<AppState>) -> AnyElement {
    row(vec![
        ghost_button(
            id,
            Some(IconName::ArrowUp),
            "Use output as input",
            cx.listener(move |this, _, window, cx| this.swap_devtools_output(section, window, cx)),
        )
        .into_any_element(),
    ])
}

fn jwt(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let mut items = vec![
        heading(
            "JWT",
            "Decode a JSON Web Token's header and payload. The signature is not verified — there \
             is no key here to check it with, so treat what it says as unproven.",
        ),
        area("devtools-jwt-in", &tools.inputs.jwt, None, None, window, cx),
    ];
    match &tools.jwt_out {
        None => {}
        Some(Err(e)) => items.push(error(e)),
        Some(Ok(token)) => {
            let (state, verdict) = token.verdict();
            let (tone, soft) = claim_tone(state);
            items.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_dot(tone, soft))
                    .child(
                        div()
                            .text_size(theme::font(Family::Chrome, Role::Body))
                            .text_color(tone)
                            .child(SharedString::from(verdict)),
                    )
                    .child(muted(&format!(
                        "· alg {}{}",
                        token.alg.as_deref().unwrap_or("?"),
                        if token.signed { "" } else { " · unsigned" }
                    )))
                    .child(
                        div()
                            .text_size(theme::font(Family::Chrome, Role::Meta))
                            .text_color(theme::warning())
                            .child("· signature not verified"),
                    )
                    .into_any_element(),
            );
            for claim in &token.claims {
                let tone = match claim.state {
                    ClaimState::Fine => None,
                    other => Some(claim_tone(other).0),
                };
                items.push(value_row(
                    format!("devtools-jwt-{}", claim.name),
                    &format!("{} ({})", claim.meaning, claim.name),
                    &claim.when,
                    tone,
                    cx,
                ));
            }
            items.push(section_label("Header").into_any_element());
            items.push(output("devtools-jwt-header", &token.header, cx));
            items.push(section_label("Payload").into_any_element());
            items.push(output("devtools-jwt-payload", &token.payload, cx));
        }
    }
    column(items)
}

fn claim_tone(state: ClaimState) -> (gpui::Rgba, gpui::Rgba) {
    match state {
        ClaimState::Fine => (theme::success(), theme::success_soft()),
        ClaimState::Expired => (theme::danger(), theme::danger_soft()),
        ClaimState::NotYet => (theme::warning(), theme::warning_soft()),
    }
}

fn time(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let mut items = vec![
        heading(
            "Date & time",
            "A Unix timestamp in seconds, milliseconds, micro- or nanoseconds, or a date in ISO \
             8601, RFC 2822 or a common written form. A date with no zone is read as local time.",
        ),
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(line(&tools.inputs.time, window, cx))
            .child(ghost_button(
                "devtools-time-now",
                Some(IconName::RefreshCw),
                "Now",
                cx.listener(|this, _, window, cx| this.devtools_now(window, cx)),
            ))
            .into_any_element(),
    ];
    match &tools.time_out {
        None => {}
        Some(Err(e)) => items.push(error(e)),
        Some(Ok((read, rows))) => {
            items.push(muted(&format!("Read as {}", read.label())));
            for (i, (label, value)) in rows.iter().enumerate() {
                items.push(value_row(
                    format!("devtools-time-{i}"),
                    label,
                    value,
                    None,
                    cx,
                ));
            }
        }
    }
    column(items)
}

fn url(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let (decode, full, plus) = (tools.url_decode, tools.url_full, tools.url_plus);
    let mut options = vec![
        pill(
            "devtools-url-encode",
            "Encode",
            !decode,
            cx.listener(move |this, _, _, cx| this.set_url_mode(false, full, plus, cx)),
        ),
        pill(
            "devtools-url-decode",
            "Decode",
            decode,
            cx.listener(move |this, _, _, cx| this.set_url_mode(true, full, plus, cx)),
        ),
        div().w(px(12.)).into_any_element(),
    ];
    if decode {
        options.push(pill(
            "devtools-url-plus",
            "+ as space",
            plus,
            cx.listener(move |this, _, _, cx| this.set_url_mode(decode, full, !plus, cx)),
        ));
    } else {
        options.push(pill(
            "devtools-url-component",
            "Component",
            !full,
            cx.listener(move |this, _, _, cx| this.set_url_mode(decode, false, plus, cx)),
        ));
        options.push(pill(
            "devtools-url-full",
            "Full URL",
            full,
            cx.listener(move |this, _, _, cx| this.set_url_mode(decode, true, plus, cx)),
        ));
    }
    let mut items = vec![
        heading(
            "URL",
            "Percent-encode text, or decode %XX escapes. A component escapes everything but \
             letters, digits and -_.~; a full URL keeps its / ? & = and the rest.",
        ),
        row(options),
        area("devtools-url-in", &tools.inputs.url, None, None, window, cx),
    ];
    match &tools.url_out {
        Ok(out) if !out.is_empty() => {
            items.push(output("devtools-url-out", out, cx));
            items.push(swap_button("devtools-url-swap", ToolSection::Url, cx));
        }
        Ok(_) => {}
        Err(e) => items.push(error(e)),
    }
    column(items)
}

fn hashes(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let expect = tools
        .inputs
        .hash_expect
        .read(cx)
        .value()
        .trim()
        .to_ascii_lowercase();
    let mut items = vec![
        heading(
            "Hash",
            "Every digest of the text at once, as lowercase hex. Paste a digest in the second \
             field to see which algorithm, if any, produced it.",
        ),
        area(
            "devtools-hash-in",
            &tools.inputs.hash,
            None,
            None,
            window,
            cx,
        ),
        line(&tools.inputs.hash_expect, window, cx),
    ];
    let mut matched = false;
    let digests = if tools.hash_out.is_empty() {
        hash::digests(b"")
    } else {
        tools.hash_out.clone()
    };
    for (name, value) in &digests {
        let hit = !expect.is_empty() && *value == expect;
        matched |= hit;
        items.push(value_row(
            format!("devtools-hash-{name}"),
            &if hit {
                format!("{name} ✓ matches")
            } else {
                name.to_string()
            },
            value,
            hit.then(theme::success),
            cx,
        ));
    }
    if !expect.is_empty() && !matched {
        items.push(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::warning())
                .child(SharedString::from(match hash::algorithm_for_hex_len(expect.len()) {
                    Some(algorithm) => format!(
                        "No match — the length says {algorithm}, but the text hashes differently"
                    ),
                    None => "No match, and no algorithm here makes a digest that long".to_string(),
                }))
                .into_any_element(),
        );
    }
    column(items)
}

fn uuid(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let (version, namespace, format) =
        (tools.uuid_version, tools.uuid_namespace, tools.uuid_format);
    let mut versions: Vec<AnyElement> = UuidVersion::ALL
        .iter()
        .map(|&v| {
            pill(
                &format!("devtools-uuid-{}", v.label()),
                v.label(),
                v == version,
                cx.listener(move |this, _, _, cx| this.set_uuid_options(v, namespace, format, cx)),
            )
        })
        .collect();
    versions.push(div().w(px(12.)).into_any_element());
    versions.push(pill(
        "devtools-uuid-case",
        if format.upper {
            "UPPERCASE"
        } else {
            "lowercase"
        },
        format.upper,
        cx.listener(move |this, _, _, cx| {
            let format = UuidFormat {
                upper: !format.upper,
                ..format
            };
            this.set_uuid_options(version, namespace, format, cx)
        }),
    ));
    versions.push(pill(
        "devtools-uuid-hyphens",
        if format.hyphens {
            "Hyphens"
        } else {
            "No hyphens"
        },
        format.hyphens,
        cx.listener(move |this, _, _, cx| {
            let format = UuidFormat {
                hyphens: !format.hyphens,
                ..format
            };
            this.set_uuid_options(version, namespace, format, cx)
        }),
    ));

    let mut items = vec![
        heading(
            "UUID",
            "v4 is random, v7 is time-ordered, v1 is time plus a random node, v5 is the SHA-1 of a \
             namespace and a name — the same pair always gives the same UUID.",
        ),
        row(versions),
    ];
    if version == UuidVersion::V5 {
        let mut spaces: Vec<AnyElement> = Namespace::ALL
            .iter()
            .map(|&ns| {
                pill(
                    &format!("devtools-uuid-ns-{}", ns.label()),
                    ns.label(),
                    ns == namespace,
                    cx.listener(move |this, _, _, cx| {
                        this.set_uuid_options(version, ns, format, cx)
                    }),
                )
            })
            .collect();
        spaces.insert(0, muted("Namespace"));
        items.push(row(spaces));
        items.push(line(&tools.inputs.uuid_name, window, cx));
    }

    let all: Vec<String> = tools
        .uuid_out
        .iter()
        .map(|id| uuids::format(id, format))
        .collect();
    let joined = all.join("\n");
    let mut actions = vec![
        primary_button(
            "devtools-uuid-generate",
            Some(IconName::RefreshCw),
            "Generate",
            cx.listener(|this, _, _, cx| this.generate_uuids(cx)),
        )
        .into_any_element(),
    ];
    if version != UuidVersion::V5 {
        actions.push(
            stepper(
                "devtools-uuid-count",
                format!("{} ×", tools.uuid_count),
                cx.listener(|this, _, _, cx| this.nudge_uuid_count(-1, cx)),
                cx.listener(|this, _, _, cx| this.nudge_uuid_count(1, cx)),
            )
            .into_any_element(),
        );
    }
    let copy_all = joined.clone();
    actions.push(
        ghost_button(
            "devtools-uuid-copy-all",
            Some(IconName::Copy),
            "Copy all",
            cx.listener(move |_, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_all.clone()));
            }),
        )
        .into_any_element(),
    );
    items.push(row(actions));
    items.push(output("devtools-uuid-out", &joined, cx));

    items.push(section_label("Inspect").into_any_element());
    items.push(line(&tools.inputs.uuid_inspect, window, cx));
    match &tools.uuid_inspect_out {
        None => {}
        Some(Err(e)) => items.push(error(e)),
        Some(Ok(rows)) => {
            for (i, (label, value)) in rows.iter().enumerate() {
                items.push(value_row(
                    format!("devtools-uuid-inspect-{i}"),
                    label,
                    value,
                    None,
                    cx,
                ));
            }
        }
    }
    column(items)
}

fn lorem(tools: &DevtoolsState, cx: &mut Context<AppState>) -> AnyElement {
    let unit = tools.lorem_unit;
    let mut options: Vec<AnyElement> = LoremUnit::ALL
        .iter()
        .map(|&u| {
            pill(
                &format!("devtools-lorem-{}", u.label()),
                u.label(),
                u == unit,
                cx.listener(move |this, _, _, cx| this.set_lorem_unit(u, cx)),
            )
        })
        .collect();
    options.push(
        stepper(
            "devtools-lorem-count",
            format!("{}", tools.lorem_count),
            cx.listener(|this, _, _, cx| this.nudge_lorem_count(-1, cx)),
            cx.listener(|this, _, _, cx| this.nudge_lorem_count(1, cx)),
        )
        .into_any_element(),
    );
    options.push(
        ghost_button(
            "devtools-lorem-again",
            Some(IconName::RefreshCw),
            "Another",
            cx.listener(|this, _, _, cx| this.reseed_lorem(cx)),
        )
        .into_any_element(),
    );
    column(vec![
        heading(
            "Lorem ipsum",
            "Placeholder text, opening on the classic sentence.",
        ),
        row(options),
        output("devtools-lorem-out", &tools.lorem_out, cx),
    ])
}

fn diff_section(tools: &DevtoolsState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let split = tools.diff_split;
    let pane = |label: &str,
                id: &'static str,
                load: &'static str,
                entity: &Entity<TextareaState>,
                target: FileTarget,
                window: &Window,
                cx: &mut Context<AppState>| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section_label(label))
                    .child(ghost_button(
                        load,
                        Some(IconName::FolderOpen),
                        "Load file…",
                        cx.listener(move |this, _, window, cx| {
                            this.pick_devtools_file(target, window, cx)
                        }),
                    )),
            )
            .child(area(id, entity, Some(target), Some(110.), window, cx))
            .into_any_element()
    };
    let left = pane(
        "Original",
        "devtools-diff-left",
        "devtools-diff-left-load",
        &tools.inputs.diff_left,
        FileTarget::DiffLeft,
        window,
        cx,
    );
    let right = pane(
        "Changed",
        "devtools-diff-right",
        "devtools-diff-right-load",
        &tools.inputs.diff_right,
        FileTarget::DiffRight,
        window,
        cx,
    );

    let mut toolbar = vec![
        pill(
            "devtools-diff-unified",
            "Unified",
            !split,
            cx.listener(|this, _, _, cx| this.set_diff_split(false, cx)),
        ),
        pill(
            "devtools-diff-split",
            "Side by side",
            split,
            cx.listener(|this, _, _, cx| this.set_diff_split(true, cx)),
        ),
        ghost_button(
            "devtools-diff-swap",
            Some(IconName::Replace),
            "Swap sides",
            cx.listener(|this, _, window, cx| this.swap_diff_sides(window, cx)),
        )
        .into_any_element(),
    ];
    let result = match &tools.diff {
        None => muted("Paste or drop two texts to compare them line by line."),
        Some(d) if d.hunks.is_empty() => muted("The two texts are identical."),
        Some(d) => {
            let (added, removed) = diff::counts(d);
            toolbar.push(mono(format!("+{added}"), theme::success()).into_any_element());
            toolbar.push(mono(format!("−{removed}"), theme::danger()).into_any_element());
            let layout = if split {
                ViewLayout::Split
            } else {
                ViewLayout::Preview
            };
            div()
                .h(px(240.))
                .flex()
                .flex_col()
                .border_1()
                .border_color(theme::border())
                .child(crate::ui::viewer::diff::render(d, layout))
                .into_any_element()
        }
    };
    items_note(
        vec![
            heading(
                "Diff",
                "Two texts compared line by line. Paste into either side, or drop or load a file.",
            ),
            div()
                .flex()
                .gap_2()
                .child(left)
                .child(right)
                .into_any_element(),
            row(toolbar),
            result,
        ],
        tools,
    )
}

/// A section's items with the file note, if any, under the inputs.
fn items_note(mut items: Vec<AnyElement>, tools: &DevtoolsState) -> AnyElement {
    if let Some(note) = note(tools) {
        items.insert(2, note);
    }
    column(items)
}
