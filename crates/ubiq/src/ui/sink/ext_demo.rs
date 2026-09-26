//! The kitchen sink's own demo registrations — invariant 9 (`X11`), M4.
//!
//! Every other item on the settings and rail containers is a conversion of something that used to
//! be a closed enum's variant (`X4`): the base's own screens, moved rather than added. **These two
//! are not that.** They are the first items on either container that arrive as a genuine
//! contribution — registered exactly the way a second edition registers its own, through
//! [`crate::ext::settings::register`] and [`crate::ext::rail::register`], with nothing in
//! `crates/ubiq/src/ext/settings.rs`, `crates/ubiq/src/ext/rail.rs`, `ui/settings.rs` or
//! `ui/rail.rs` touched to add them.
//!
//! **Why this module rather than either of those.** The demo needs no project and no host call —
//! exactly the sink's own rule — so its code lives beside the sink's, the one screen already built
//! to have nothing behind it. The switch that gates the demo mode is
//! [`crate::state::sink::SinkState::ext_demo_on`], toggled from the demo settings section below and
//! read by the demo rail mode's [`Availability::When`] predicate: turning it off drops the mode out
//! of the rail entirely, in place, which is what "a broken container shows as broken" means for a
//! `When` contribution — break the predicate or the resolve step and the mode's presence stops
//! tracking the switch, or vanishes outright.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div};

use crate::app::AppState;
use crate::ext::rail::{Availability, RailModeSpec, register as register_rail};
use crate::ext::settings::{
    SectionCtx, SectionGate, SettingsContainer, SettingsSectionSpec, register as register_settings,
};
use crate::ext::{Registry, ids};
use crate::state::ui_id;
use crate::theme::{Family, Role};
use crate::ui::empty;
use crate::ui::kit::{UbiqIcon, check_box, column, heading, setting_row};

/// Registers the demo settings section into the application overlay.
///
/// Lands in the same group the base's own system-level sections do (`ids::SETTINGS_APP_SYSTEM`) —
/// a contribution picks one of the container's declared groups, it never invents one.
pub fn settings_section(reg: &mut Registry<SettingsSectionSpec>) {
    register_settings(
        reg,
        SettingsSectionSpec {
            id: ids::EXT_DEMO_SETTINGS,
            container: SettingsContainer::App,
            group: ids::SETTINGS_APP_SYSTEM,
            label: "Extensions demo",
            icon: || UbiqIcon::ModeSink.into(),
            gate: SectionGate::Always,
            count: None,
            on_show: None,
            render: render_section,
        },
    );
}

fn render_section(
    ctx: &SectionCtx<'_>,
    _window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let on = ctx.app.sink.ext_demo_on;
    column(vec![
        heading(
            "Extensions demo",
            "Invariant 9: a container with no contributor is silently dead. This section and the \
             rail mode its switch gates are the proof — both are registered exactly the way a \
             second edition registers its own, through `ext::settings::register` and \
             `ext::rail::register`, with neither container's own module touched to add them.",
        ),
        setting_row(
            "Demo rail mode",
            "Switches the rail's own demo mode on the APP row. It is gated by \
             `Availability::When` — a predicate reading this very switch, answered from interface \
             state and needing no message to the host, the base's own ten having never exercised \
             the conditional case.",
            check_box(
                "ext-demo-toggle",
                on,
                cx.listener(|this, _, _, cx| this.toggle_ext_demo(cx)),
            )
            .into_any_element(),
        ),
    ])
}

/// Registers the demo rail mode.
///
/// `Availability::When` reads [`crate::state::sink::SinkState::ext_demo_on`] — the switch the
/// section above draws — so the mode is on the rail only while the switch is, and drops out
/// entirely rather than being drawn disabled: that is `Availability::When`'s whole point, and the
/// base's own ten never exercised it (`D184`).
pub fn rail_mode(reg: &mut Registry<RailModeSpec>) {
    register_rail(
        reg,
        RailModeSpec {
            id: ids::EXT_DEMO_RAIL,
            group: ids::RAIL_APP,
            label: "Extensions demo",
            note: "A rail mode contributed through `ext::rail`, on only while Settings \u{203a} \
                   System \u{203a} Extensions demo is switched on.",
            slug: "ext-demo",
            icon: || UbiqIcon::ModeSink.into(),
            ui_id: ui_id::RAIL_MODE_EXT_DEMO,
            availability: Availability::When(|app, _cx| app.sink.ext_demo_on),
            has_pane_region: false,
            needs_project: false,
            opens_left: false,
            opens_right: false,
            centre: Some(render_centre),
            default_layout: None,
            // Not a place: the sink it lives beside is not addressable either.
            destination: None,
            furniture: None,
            side_furniture: None,
            on_enter: None,
        },
    );
}

fn render_centre(_app: &AppState, _window: &mut Window, _cx: &mut Context<AppState>) -> AnyElement {
    empty::empty_page(
        "Extensions demo",
        "A rail mode contributed through `ext::rail::register`, gated by `Availability::When` — \
         on only while Settings \u{203a} System \u{203a} Extensions demo is switched on. Turn it \
         off there and this mode drops out of the rail entirely, in place.",
        UbiqIcon::ModeSink,
        Some(
            div()
                .text_size(crate::theme::font(Family::Chrome, Role::Meta))
                .text_color(crate::theme::text_faint())
                .child("Contributed through ext::rail::register — invariant 9's proof.")
                .into_any_element(),
        ),
    )
    .into_any_element()
}
