//! The "Diagrams" settings section (App > System, `settings.md`): the quality override, the palette
//! and a motion placeholder. The values are the interface's copy of `proto::Settings`; the host
//! keeps them in //! nothing lands in a base-parsed struct.
//!
//! The quality override is read by whoever compiles a document (`compile::Options::quality`, D15);
//! the palette is the visual preset, one for every diagram or each file's own (D11, D38); motion off is the reduced-motion
//! state: the camera is instant (`motion.rs`, P7.2) and so are the fades, and the comets are static
//! (`animate.rs`, P7.3).

use gpui::{AnyElement, Context, IntoElement, Window};
use gpui_component::{Icon, IconName};
use crate::app::AppState;
use crate::ext::settings::{SectionCtx, SectionGate, SettingsContainer, SettingsSectionSpec};
use crate::ext::{Registry, ids};
use crate::ui::eid;
use crate::ui::kit::{check_box, choice_pill, column, heading, setting_row};

use crate::state::archify::ui;

/// The override choices: label, and the profile it names (`None` follows the document).
pub const QUALITY: [(&str, Option<&str>); 4] = [
    ("Document", None),
    ("Advisory", Some("advisory")),
    ("Standard", Some("standard")),
    ("Showcase", Some("showcase")),
];

/// The palettes there are (D11, D38): the preset each file names, or one for every diagram. The
/// values are `Preset::as_str`, and `document` for the first.
pub const PALETTES: [(&str, &str); 5] = [
    ("Document", "document"),
    ("Classic", "classic"),
    ("Signal flow", "signal-flow"),
    ("Blueprint", "blueprint"),
    ("Editorial", "editorial"),
];

pub fn register(reg: &mut Registry<SettingsSectionSpec>) {
    crate::ext::settings::register(
        reg,
        SettingsSectionSpec {
            id: ids::ARCHIFY_SETTINGS,
            container: SettingsContainer::App,
            group: ids::SETTINGS_APP_SYSTEM,
            label: "Diagrams",
            icon: || Icon::new(IconName::LayoutDashboard),
            gate: SectionGate::Always,
            count: None,
            on_show: None,
            render,
        },
    );
}

fn render(_ctx: &SectionCtx<'_>, _window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let settings = ui(cx).settings.clone();

    let quality = QUALITY.map(|(label, value)| {
        choice_pill(
            eid("archify-quality", label),
            label,
            settings.quality.as_deref() == value,
            cx.listener(move |app, _, _window, cx| {
                app.update_archify_settings(|s| s.quality = value.map(str::to_string), cx);
                cx.refresh_windows();
            }),
        )
    });
    let palette = PALETTES.map(|(label, value)| {
        choice_pill(
            eid("archify-palette", label),
            label,
            settings.palette == value,
            cx.listener(move |app, _, _window, cx| {
                app.update_archify_settings(|s| s.palette = value.to_string(), cx);
                cx.refresh_windows();
            }),
        )
    });

    column(vec![
        heading(
            "Diagrams",
            "How Archify diagrams are checked and drawn. Kept in the Diagrams store, not in the \
             project.",
        ),
        setting_row(
            "Quality profile",
            "Overrides the severity of the layout gates for every diagram. \u{201c}Document\u{201d} \
             follows each file's own meta.quality_profile.",
            pills(quality),
        ),
        setting_row(
            "Palette",
            "The visual preset. \u{201c}Document\u{201d} follows each file's own meta.visual_preset \
             (classic when it names none). Classic keeps the Ubiq chrome around Archify's kind \
             colours; the others are Archify's own tables and take the whole picture.",
            pills(palette),
        ),
        setting_row(
            "Motion",
            "Off: the camera jumps to a node instead of gliding, fades are instant and the trace \
             comets stand still (the OS reduced-motion setting does the same).",
            check_box(
                "archify-motion",
                settings.motion,
                cx.listener(|app, _, _window, cx| {
                    app.update_archify_settings(|s| s.motion = !s.motion, cx);
                    cx.refresh_windows();
                }),
            )
            .into_any_element(),
        ),
    ])
}

/// A row of pills.
fn pills<const N: usize>(pills: [gpui::Stateful<gpui::Div>; N]) -> AnyElement {
    use gpui::{ParentElement, Styled};
    gpui::div()
        .flex()
        .gap_1()
        .children(pills)
        .into_any_element()
}
